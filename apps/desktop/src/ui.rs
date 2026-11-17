use crate::backend::{self, RuntimeSession, Snapshot};
use gpui::{
    App, Application, Background, Bounds, BoxShadow, ClipboardItem, Context, Div, ElementId,
    FocusHandle, FontWeight, KeyBinding, SharedString, Stateful, TitlebarOptions, Window,
    WindowBackgroundAppearance, WindowBounds, WindowControlArea, WindowHandle, WindowOptions,
    actions, div, linear_color_stop, linear_gradient, point, prelude::*, px, rgb, rgba, size,
};
use hyperrev_transport::Readiness;
use std::{cell::RefCell, rc::Rc, time::Duration};

const APP_TITLE: &str = "External Everything";
const INK: u32 = 0x24334b;
const MUTED: u32 = 0x4f6079;
actions!(external_everything, [FocusNext, FocusPrevious]);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RouteChoice {
    Setup,
    RestartRequired,
    Games,
}

// Only the live attachment probe opens the chooser. Installation metadata is
// useful for the restart screen, never as a substitute for attachment readiness.
fn route_choice(snapshot: &Snapshot) -> RouteChoice {
    if matches!(&snapshot.attachment, Readiness::Ready { cr3 } if cr3 & 0x000f_ffff_ffff_f000 != 0)
    {
        RouteChoice::Games
    } else if snapshot.installed {
        RouteChoice::RestartRequired
    } else {
        RouteChoice::Setup
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WindowRole {
    Loader,
    Chooser,
}

impl WindowRole {
    fn title(self) -> &'static str {
        match self {
            Self::Loader => APP_TITLE,
            Self::Chooser => "External Everything · Games",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    Checking,
    Setup,
    RestartRequired,
    Games,
    Error,
}

impl From<RouteChoice> for Stage {
    fn from(choice: RouteChoice) -> Self {
        match choice {
            RouteChoice::Setup => Self::Setup,
            RouteChoice::RestartRequired => Self::RestartRequired,
            RouteChoice::Games => Self::Games,
        }
    }
}

#[derive(Clone, Copy)]
enum Operation {
    Inspect,
    Setup,
    Restore,
}

impl Operation {
    fn perform(self) -> anyhow::Result<Snapshot> {
        match self {
            Self::Inspect => backend::inspect(),
            Self::Setup => backend::setup(),
            Self::Restore => backend::restore(),
        }
    }

    fn status(self) -> &'static str {
        match self {
            Self::Inspect => "Checking…",
            Self::Setup => "Setting up…",
            Self::Restore => "Restoring…",
        }
    }
}

#[derive(Clone)]
enum Action {
    Setup,
    Restore,
    Inspect,
    Games,
    Back,
    Open(String),
    Advanced,
    Copy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Caption {
    Minimize,
    Maximize,
    Close,
}

impl Caption {
    fn area(self) -> WindowControlArea {
        match self {
            Self::Minimize => WindowControlArea::Min,
            Self::Maximize => WindowControlArea::Max,
            Self::Close => WindowControlArea::Close,
        }
    }

    fn glyph(self, maximized: bool) -> &'static str {
        match self {
            Self::Minimize => "\u{e921}",
            Self::Maximize if maximized => "\u{e923}",
            Self::Maximize => "\u{e922}",
            Self::Close => "\u{e8bb}",
        }
    }

    fn label(self, maximized: bool) -> &'static str {
        match self {
            Self::Minimize => "Minimize",
            Self::Maximize if maximized => "Restore window",
            Self::Maximize => "Maximize",
            Self::Close => "Close",
        }
    }

    fn enabled(self, busy: bool) -> bool {
        self != Self::Close || !busy
    }
}

fn gradient(from: u32, to: u32) -> Background {
    linear_gradient(
        165.,
        linear_color_stop(rgba(from), 0.),
        linear_color_stop(rgba(to), 1.),
    )
}

fn elevation(color: u32, offset: f32, blur: f32) -> BoxShadow {
    BoxShadow {
        color: rgba(color).into(),
        offset: point(px(0.), px(offset)),
        blur_radius: px(blur),
        spread_radius: px(-2.),
    }
}

struct CaptionTip(&'static str);

impl Render for CaptionTip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_2()
            .py_1()
            .rounded(px(6.))
            .bg(rgb(0x26344b))
            .text_color(rgb(0xffffff))
            .font_family("Segoe UI")
            .text_size(px(11.))
            .child(self.0)
    }
}

struct MainView {
    role: WindowRole,
    stage: Stage,
    snapshot: Option<Snapshot>,
    focus: FocusHandle,
    busy: bool,
    advanced: bool,
    failed: bool,
    status: SharedString,
    details: SharedString,
    session: Option<RuntimeSession>,
}

impl MainView {
    fn new(
        role: WindowRole,
        snapshot: Option<Snapshot>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus = cx.focus_handle();
        focus.focus(window);
        let mut view = Self {
            role,
            stage: Stage::Checking,
            snapshot: None,
            focus,
            busy: false,
            advanced: false,
            failed: false,
            status: "Checking…".into(),
            details: "".into(),
            session: None,
        };
        if let Some(snapshot) = snapshot {
            view.accept_snapshot(snapshot);
        }
        view
    }

    fn accept_snapshot(&mut self, snapshot: Snapshot) {
        self.stage = route_choice(&snapshot).into();
        self.failed = false;
        self.status = match self.stage {
            Stage::Setup => "Setup required",
            Stage::RestartRequired => "Restart Windows",
            Stage::Games if self.role == WindowRole::Loader => "Ready",
            _ => "",
        }
        .into();
        self.details = snapshot_details(&snapshot).into();
        self.snapshot = Some(snapshot);
    }

    fn show_error(&mut self, error: String) {
        self.failed = true;
        self.status = brief_error(&error).into();
        self.details = error.into();
    }

    fn dispatch(&mut self, action: Action, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        match action {
            Action::Advanced => {
                self.advanced = !self.advanced;
                cx.notify();
            }
            Action::Copy => {
                cx.write_to_clipboard(ClipboardItem::new_string(self.details.to_string()));
            }
            Action::Open(game_id) => self.launch(game_id, window, cx),
            Action::Setup if self.session.is_none() => self.perform(Operation::Setup, window, cx),
            Action::Restore if self.session.is_none() => {
                self.perform(Operation::Restore, window, cx);
            }
            Action::Inspect if self.session.is_none() => {
                self.perform(Operation::Inspect, window, cx);
            }
            Action::Games if self.stage == Stage::Games => {
                self.change_window(WindowRole::Chooser, window, cx);
            }
            Action::Back if self.session.is_none() => {
                self.change_window(WindowRole::Loader, window, cx);
            }
            _ => {}
        }
    }

    fn perform(&mut self, operation: Operation, window: &Window, cx: &mut Context<Self>) {
        self.perform_with_notice(operation, None, window, cx);
    }

    fn perform_with_notice(
        &mut self,
        operation: Operation,
        notice: Option<String>,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        self.busy = true;
        self.failed = false;
        self.status = operation.status().into();
        let worker = cx
            .background_executor()
            .spawn(async move { operation.perform().map_err(|error| format!("{error:#}")) });
        cx.spawn_in(window, async move |view, cx| {
            let result = worker.await;
            let _ = view.update_in(cx, |view, window, cx| {
                view.busy = false;
                match result {
                    Ok(snapshot) => {
                        view.accept_snapshot(snapshot);
                        if let Some(notice) = notice {
                            let details = format!("{}\n\n{notice}", view.details);
                            if view.stage == Stage::Games {
                                view.show_error(notice);
                            } else {
                                view.failed = true;
                            }
                            view.details = details.into();
                        }
                        match (view.role, view.stage) {
                            (WindowRole::Loader, Stage::Games) => {
                                view.change_window(WindowRole::Chooser, window, cx);
                            }
                            (WindowRole::Chooser, Stage::Setup | Stage::RestartRequired) => {
                                view.change_window(WindowRole::Loader, window, cx);
                            }
                            _ => {}
                        }
                    }
                    Err(error) => {
                        view.snapshot = None;
                        view.stage = Stage::Error;
                        view.role = WindowRole::Loader;
                        window.set_window_title(APP_TITLE);
                        view.show_error(match notice {
                            Some(notice) => format!("{notice}\n\nRecheck: {error}"),
                            None => error,
                        });
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn change_window(&mut self, role: WindowRole, window: &mut Window, cx: &mut Context<Self>) {
        let Some(snapshot) = self.snapshot.clone() else {
            return;
        };
        if role == WindowRole::Chooser && route_choice(&snapshot) != RouteChoice::Games {
            return;
        }
        // The existing real window stays alive if the replacement fails to open.
        // A successful handoff always opens first, then removes the previous root.
        match open_window(role, Some(snapshot), cx) {
            Ok(opened) => {
                if self.failed {
                    let status = self.status.clone();
                    let details = self.details.clone();
                    let _ = opened.update(cx, |view, _, cx| {
                        view.failed = true;
                        view.status = status;
                        view.details = details;
                        cx.notify();
                    });
                }
                window.remove_window();
            }
            Err(error) => {
                self.show_error(format!("Opening window: {error:#}"));
                cx.notify();
            }
        }
    }

    fn launch(&mut self, game_id: String, window: &Window, cx: &mut Context<Self>) {
        if self.session.is_some() || self.stage != Stage::Games {
            return;
        }
        let Some(game) = self
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.games.iter().find(|game| game.id == game_id))
            .filter(|game| game.available())
        else {
            return;
        };
        let name = game.name.clone();
        self.busy = true;
        self.failed = false;
        self.status = "Opening…".into();
        // The backend re-probes attachment and checks the executable/hash. UI
        // metadata only enables the button; it never authorizes a child launch.
        let worker = cx
            .background_executor()
            .spawn(async move { backend::launch(&game_id).map_err(|error| format!("{error:#}")) });
        cx.spawn_in(window, async move |view, cx| {
            let result = worker.await;
            let _ = view.update_in(cx, |view, window, cx| {
                view.busy = false;
                match result {
                    Ok(session) => {
                        view.details = format!("Game: {}\nRunning: {name}", session.game_id).into();
                        view.session = Some(session);
                        view.status = format!("Running: {name}").into();
                        view.monitor_session(window, cx);
                    }
                    Err(error) => view.show_error(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn monitor_session(&self, window: &Window, cx: &mut Context<Self>) {
        let executor = cx.background_executor().clone();
        cx.spawn_in(window, async move |view, cx| {
            loop {
                executor.timer(Duration::from_millis(500)).await;
                let keep_running = view
                    .update_in(cx, |view, window, cx| {
                        let Some(session) = view.session.as_mut() else {
                            return false;
                        };
                        match session.try_wait() {
                            Ok(None) => true,
                            result => {
                                // RuntimeSession owns child cleanup when this
                                // view closes or a monitoring error drops it.
                                view.session.take();
                                let notice = match result {
                                    Ok(Some(status)) if status.success() => None,
                                    Ok(Some(status)) => Some(format!("Game exited: {status}")),
                                    Err(error) => Some(format!("Game status: {error:#}")),
                                    Ok(None) => unreachable!(),
                                };
                                // Re-probe on return before enabling another
                                // launch; retain a failed exit in the details.
                                view.perform_with_notice(Operation::Inspect, notice, window, cx);
                                // The chooser remains a real window throughout
                                // the child session; bring it back on return.
                                window.activate_window();
                                view.focus.focus(window);
                                cx.activate(false);
                                cx.notify();
                                false
                            }
                        }
                    })
                    .unwrap_or(false);
                if !keep_running {
                    break;
                }
            }
        })
        .detach();
    }

    fn button(
        &self,
        id: impl Into<ElementId>,
        label: &'static str,
        action: Action,
        primary: bool,
        enabled: bool,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        let enabled = enabled && !self.busy;
        let keyboard_action = action.clone();
        div()
            .id(id)
            .flex()
            .items_center()
            .justify_center()
            .h(px(44.))
            .px_6()
            .rounded_full()
            .border_1()
            .border_color(rgba(0xffffffd9))
            .bg(if primary {
                gradient(0x2365dbff, 0x124fc6f0)
            } else {
                gradient(0xffffffdc, 0xffffff70)
            })
            .shadow(vec![elevation(
                if primary { 0x205dc53b } else { 0x2a42621f },
                5.,
                14.,
            )])
            .text_color(rgb(if primary { 0xffffff } else { INK }))
            .text_size(px(13.))
            .font_weight(FontWeight::SEMIBOLD)
            .child(label)
            .when(!enabled, |button| button.opacity(0.45))
            .when(enabled, |button| {
                button
                    .focusable()
                    .tab_stop(true)
                    .tab_index(0)
                    .cursor_pointer()
                    .hover(move |style| {
                        style.bg(if primary {
                            gradient(0x195bd7ff, 0x1648bafc)
                        } else {
                            gradient(0xfffffff5, 0xffffffb8)
                        })
                    })
                    .active(move |style| {
                        style.bg(if primary {
                            gradient(0x144abaff, 0x123da5ff)
                        } else {
                            gradient(0xe5edfbf0, 0xffffffd0)
                        })
                    })
                    .focus(|style| style.border_color(rgb(0x1849ac)).border_2())
                    .on_click(cx.listener(move |view, _, window, cx| {
                        view.dispatch(action.clone(), window, cx);
                    }))
                    .on_key_down(cx.listener(
                        move |view, event: &gpui::KeyDownEvent, window, cx| {
                            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                                cx.stop_propagation();
                                view.dispatch(keyboard_action.clone(), window, cx);
                            }
                        },
                    ))
            })
    }

    fn small_button(
        &self,
        id: &'static str,
        label: &'static str,
        action: Action,
        enabled: bool,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        self.button(id, label, action, false, enabled, cx)
            .h(px(28.))
            .px_3()
            .border_0()
            .shadow(Vec::new())
            .bg(rgba(0xffffff00))
            .font_weight(FontWeight::MEDIUM)
            .text_size(px(11.))
            .text_color(rgb(MUTED))
    }

    fn loader(&self, cx: &Context<Self>) -> Div {
        let (label, action) = match self.stage {
            Stage::Checking | Stage::Setup => ("Setup", Action::Setup),
            Stage::RestartRequired | Stage::Error => ("Recheck", Action::Inspect),
            Stage::Games => ("Games", Action::Games),
        };
        div()
            .flex()
            .flex_col()
            .items_center()
            .gap_4()
            .child(
                div()
                    .text_size(px(28.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(APP_TITLE),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_4()
                    .p_5()
                    .rounded(px(28.))
                    .border_1()
                    .border_color(rgba(0xffffffdb))
                    .bg(gradient(0xffffff9c, 0xffffff3d))
                    .shadow(vec![
                        elevation(0x354f7826, 10., 30.),
                        elevation(0xffffff80, -1., 2.),
                    ])
                    .child(
                        div()
                            .max_w(px(340.))
                            .text_center()
                            .text_color(rgb(if self.failed { 0x9b2442 } else { MUTED }))
                            .child(self.status.clone()),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_3()
                            .child(
                                self.button(
                                    "loader-primary",
                                    label,
                                    action,
                                    true,
                                    self.stage != Stage::Checking,
                                    cx,
                                )
                                .w(px(160.)),
                            )
                            .child(
                                self.button(
                                    "restore-button",
                                    "Restore",
                                    Action::Restore,
                                    false,
                                    self.stage != Stage::Checking,
                                    cx,
                                )
                                .w(px(120.)),
                            ),
                    ),
            )
            .when(
                !matches!(self.stage, Stage::RestartRequired | Stage::Error),
                |view| {
                    view.child(self.small_button("recheck", "Recheck", Action::Inspect, true, cx))
                },
            )
    }

    fn games(&self, cx: &Context<Self>) -> Div {
        let running = self.session.is_some();
        let games = self
            .snapshot
            .as_ref()
            .map(|snapshot| snapshot.games.as_slice())
            .unwrap_or(&[]);
        let mut list = div().flex().flex_col().gap_3().w_full();
        if games.is_empty() {
            list = list.child(
                div()
                    .py_6()
                    .text_center()
                    .text_color(rgb(MUTED))
                    .child("No games installed"),
            );
        }
        for (index, game) in games.iter().enumerate() {
            list = list.child(
                div()
                    .id(("game", index))
                    .flex()
                    .items_center()
                    .gap_3()
                    .w_full()
                    .p_4()
                    .rounded(px(20.))
                    .border_1()
                    .border_color(rgba(0xffffffdb))
                    .bg(gradient(0xffffff9c, 0xffffff3d))
                    .child(div().flex_1().min_w_0().child(game.name.clone()))
                    .when(game.available(), |row| {
                        row.child(
                            self.button(
                                ("open-game", index),
                                "Open",
                                Action::Open(game.id.clone()),
                                true,
                                !running && self.stage == Stage::Games,
                                cx,
                            )
                            .h(px(34.))
                            .px_5(),
                        )
                    })
                    .when(!game.available(), |row| {
                        row.child(
                            div()
                                .text_size(px(11.))
                                .text_color(rgb(MUTED))
                                .child("Coming later"),
                        )
                    }),
            );
        }
        div()
            .flex()
            .flex_col()
            .items_center()
            .gap_4()
            .w_full()
            .max_w(px(440.))
            .child(list)
            .when(!self.status.is_empty(), |view| {
                view.child(
                    div()
                        .text_size(px(12.))
                        .text_center()
                        .text_color(rgb(if self.failed { 0x9b2442 } else { MUTED }))
                        .child(self.status.clone()),
                )
            })
            .child(
                div()
                    .flex()
                    .gap_3()
                    .child(self.small_button("back", "Back", Action::Back, !running, cx))
                    .child(self.small_button("refresh", "Refresh", Action::Inspect, !running, cx)),
            )
    }

    fn advanced(&self, cx: &Context<Self>) -> Div {
        div()
            .flex()
            .flex_col()
            .gap_2()
            .w_full()
            .max_w(px(440.))
            .p_4()
            .rounded(px(18.))
            .border_1()
            .border_color(rgba(0xffffffbf))
            .bg(rgba(0xffffff8c))
            .child(
                div()
                    .id("diagnostics")
                    .max_h(px(180.))
                    .w_full()
                    .overflow_y_scroll()
                    .font_family("Consolas")
                    .text_size(px(10.))
                    .whitespace_normal()
                    .child(self.details.clone()),
            )
            .child(self.small_button("copy-details", "Copy details", Action::Copy, true, cx))
    }

    fn caption(&self, control: Caption, window: &Window) -> Stateful<Div> {
        let maximized = window.is_maximized();
        let label = control.label(maximized);
        // Passive native hit regions preserve snap and true maximize/restore.
        // Neither these controls nor any header ancestor tracks keyboard focus.
        div()
            .id(match control {
                Caption::Minimize => "caption-minimize",
                Caption::Maximize => "caption-maximize",
                Caption::Close => "caption-close",
            })
            .window_control_area(control.area())
            .w(px(34.))
            .h(px(28.))
            .flex()
            .items_center()
            .justify_center()
            .rounded_full()
            .font_family("Segoe MDL2 Assets")
            .text_size(px(10.))
            .text_color(rgb(INK))
            .child(control.glyph(maximized))
            .when(!control.enabled(self.busy), |button| button.opacity(0.4))
            .hover(move |style| {
                if control == Caption::Close {
                    style.bg(rgba(0xea4764e8)).text_color(rgb(0xffffff))
                } else {
                    style.bg(rgba(0xffffffb8))
                }
            })
            .tooltip(move |_, cx| cx.new(|_| CaptionTip(label)).into())
    }

    fn titlebar(&self, window: &Window) -> Div {
        div()
            .h(px(52.))
            .w_full()
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap_3()
            .pt(px(8.))
            .pb(px(6.))
            .px(px(12.))
            .child(
                div()
                    .id("window-drag")
                    .window_control_area(WindowControlArea::Drag)
                    .flex_1()
                    .h_full()
                    .flex()
                    .items_center()
                    .gap_2()
                    .pl_2()
                    .child(
                        div()
                            .size(px(22.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(7.))
                            .border_1()
                            .border_color(rgba(0xffffffb3))
                            .bg(gradient(0x3477e6ff, 0x174dbdff))
                            .text_color(rgb(0xffffff))
                            .text_size(px(13.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("E"),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(rgb(if window.is_window_active() {
                                INK
                            } else {
                                MUTED
                            }))
                            .child(if self.role == WindowRole::Chooser {
                                "Games"
                            } else {
                                APP_TITLE
                            }),
                    ),
            )
            .child(
                div()
                    .flex()
                    .gap_1()
                    .p(px(3.))
                    .rounded_full()
                    .border_1()
                    .border_color(rgba(0xffffffc7))
                    .bg(gradient(0xffffffbd, 0xffffff54))
                    .shadow(vec![elevation(0x354d721c, 3., 10.)])
                    .child(self.caption(Caption::Minimize, window))
                    .child(self.caption(Caption::Maximize, window))
                    .child(self.caption(Caption::Close, window)),
            )
    }
}

impl Render for MainView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("glass-window")
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .font_family("Segoe UI")
            .text_size(px(12.))
            .text_color(rgb(INK))
            .bg(gradient(0xe5efffdc, 0xf6edf9d4))
            .when(!window.is_maximized(), |view| view.rounded(px(12.)))
            .child(
                div()
                    .absolute()
                    .size(px(250.))
                    .top(px(-170.))
                    .right(px(-100.))
                    .rounded_full()
                    .bg(rgba(0xb8c4ff25))
                    .shadow(vec![elevation(0xb8c4ff38, 0., 90.)]),
            )
            .child(
                div()
                    .absolute()
                    .size(px(220.))
                    .bottom(px(-170.))
                    .left(px(-70.))
                    .rounded_full()
                    .bg(rgba(0x8ccaff24))
                    .shadow(vec![elevation(0x8ccaff38, 0., 95.)]),
            )
            .child(self.titlebar(window))
            // Content alone owns focus so native caption MouseDown is passive.
            .child(
                div()
                    .id("content")
                    .key_context("ExternalEverything")
                    .track_focus(&self.focus)
                    .on_action(cx.listener(|_, _: &FocusNext, window, _| window.focus_next()))
                    .on_action(cx.listener(|_, _: &FocusPrevious, window, _| window.focus_prev()))
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_3()
                    .p_6()
                    .overflow_y_scroll()
                    .when(!self.advanced, |view| view.justify_center())
                    .child(match self.role {
                        WindowRole::Loader => self.loader(cx),
                        WindowRole::Chooser => self.games(cx),
                    })
                    .child(self.small_button(
                        "advanced",
                        if self.advanced {
                            "Hide advanced"
                        } else {
                            "Advanced"
                        },
                        Action::Advanced,
                        true,
                        cx,
                    ))
                    .when(self.advanced, |view| view.child(self.advanced(cx))),
            )
    }
}

fn snapshot_details(snapshot: &Snapshot) -> String {
    let attachment = match &snapshot.attachment {
        Readiness::Ready { cr3 } => format!("Ready (CR3: 0x{cr3:016x})"),
        Readiness::Absent { response } => format!("Absent (response: 0x{response:016x})"),
    };
    format!(
        "Attachment: {attachment}\nInstalled: {}\nGames: {}",
        snapshot.installed,
        snapshot.games.len(),
    )
}

fn brief_error(error: &str) -> String {
    let line = error
        .lines()
        .next()
        .filter(|line| !line.trim().is_empty())
        .unwrap_or("Operation failed")
        .trim();
    let shortened: String = line.chars().take(110).collect();
    if line.chars().count() > 110 {
        format!("{shortened}…")
    } else {
        shortened
    }
}

fn open_window(
    role: WindowRole,
    snapshot: Option<Snapshot>,
    cx: &mut App,
) -> anyhow::Result<WindowHandle<MainView>> {
    let bounds = Bounds::centered(None, size(px(560.), px(360.)), cx);
    let opened = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            window_min_size: Some(size(px(480.), px(340.))),
            window_background: WindowBackgroundAppearance::Blurred,
            titlebar: Some(TitlebarOptions {
                title: Some(role.title().into()),
                appears_transparent: true,
                ..Default::default()
            }),
            app_id: Some("external-everything".into()),
            ..Default::default()
        },
        move |window, cx| {
            let inspect = snapshot.is_none();
            let view = cx.new(|cx| {
                let mut view = MainView::new(role, snapshot, window, cx);
                if inspect {
                    view.perform(Operation::Inspect, window, cx);
                }
                view
            });
            let weak = view.downgrade();
            window.on_window_should_close(cx, move |_, cx| {
                weak.update(cx, |view, _| Caption::Close.enabled(view.busy))
                    .unwrap_or(true)
            });
            window.activate_window();
            view
        },
    )?;
    cx.activate(true);
    Ok(opened)
}

pub fn run() -> anyhow::Result<()> {
    let failure = Rc::new(RefCell::new(None));
    let failure_slot = failure.clone();
    Application::new().run(move |cx: &mut App| {
        cx.bind_keys([
            KeyBinding::new("tab", FocusNext, Some("ExternalEverything")),
            KeyBinding::new("shift-tab", FocusPrevious, Some("ExternalEverything")),
        ]);
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        if let Err(error) = open_window(WindowRole::Loader, None, cx) {
            *failure_slot.borrow_mut() = Some(error);
            cx.quit();
        }
    });
    match failure.borrow_mut().take() {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::Game;
    use std::path::PathBuf;

    fn snapshot(attachment: Readiness, installed: bool) -> Snapshot {
        Snapshot {
            attachment,
            installed,
            games: Vec::new(),
        }
    }

    fn game(executable: Option<PathBuf>) -> Game {
        Game {
            id: "future".into(),
            name: "Future Game".into(),
            executable,
            sha256: None,
            embedded: false,
        }
    }

    #[test]
    fn absent_without_install_routes_to_setup() {
        assert_eq!(
            route_choice(&snapshot(Readiness::Absent { response: 0 }, false)),
            RouteChoice::Setup
        );
    }

    #[test]
    fn installed_without_attachment_requires_restart() {
        assert_eq!(
            route_choice(&snapshot(Readiness::Absent { response: 0 }, true)),
            RouteChoice::RestartRequired
        );
    }

    #[test]
    fn live_ready_without_install_opens_games() {
        assert_eq!(
            route_choice(&snapshot(Readiness::Ready { cr3: 0x1000 }, false)),
            RouteChoice::Games
        );
    }

    #[test]
    fn live_ready_with_install_opens_games() {
        assert_eq!(
            route_choice(&snapshot(Readiness::Ready { cr3: 0x1000 }, true)),
            RouteChoice::Games
        );
    }

    #[test]
    fn zero_cr3_without_install_stays_on_setup() {
        assert_eq!(
            route_choice(&snapshot(Readiness::Ready { cr3: 0 }, false)),
            RouteChoice::Setup
        );
    }

    #[test]
    fn zero_cr3_with_install_stays_on_restart() {
        assert_eq!(
            route_choice(&snapshot(Readiness::Ready { cr3: 0 }, true)),
            RouteChoice::RestartRequired
        );
    }

    #[test]
    fn cr3_flags_without_a_frame_never_open_games() {
        assert_eq!(
            route_choice(&snapshot(Readiness::Ready { cr3: 0xfff }, false)),
            RouteChoice::Setup
        );
        assert_eq!(
            route_choice(&snapshot(Readiness::Ready { cr3: 0xfff }, true)),
            RouteChoice::RestartRequired
        );
    }

    #[test]
    fn nonzero_absent_response_never_becomes_ready() {
        assert_eq!(
            route_choice(&snapshot(Readiness::Absent { response: 0x1234 }, false)),
            RouteChoice::Setup
        );
    }

    #[test]
    fn empty_catalog_is_valid_for_ready_chooser() {
        let snapshot = snapshot(Readiness::Ready { cr3: 0x4000 }, false);
        assert!(snapshot.games.is_empty());
        assert_eq!(route_choice(&snapshot), RouteChoice::Games);
    }

    #[test]
    fn planned_game_metadata_never_grants_attachment_readiness() {
        let mut snapshot = snapshot(Readiness::Absent { response: 0 }, false);
        snapshot.games.push(game(None));
        assert_eq!(route_choice(&snapshot), RouteChoice::Setup);
    }

    #[test]
    fn executable_metadata_never_skips_required_restart() {
        let mut snapshot = snapshot(Readiness::Absent { response: 0 }, true);
        snapshot.games.push(game(Some(PathBuf::from("future.exe"))));
        assert_eq!(route_choice(&snapshot), RouteChoice::RestartRequired);
    }

    #[test]
    fn planned_modules_remain_visible_in_a_ready_catalog() {
        let mut snapshot = snapshot(Readiness::Ready { cr3: 0x8000 }, true);
        snapshot.games.push(game(None));
        assert_eq!(route_choice(&snapshot), RouteChoice::Games);
        assert_eq!(snapshot.games[0].name, "Future Game");
    }

    #[test]
    fn caption_controls_remain_native() {
        assert_eq!(Caption::Minimize.area(), WindowControlArea::Min);
        assert_eq!(Caption::Maximize.area(), WindowControlArea::Max);
        assert_eq!(Caption::Close.area(), WindowControlArea::Close);
        assert_ne!(
            Caption::Maximize.glyph(false),
            Caption::Maximize.glyph(true)
        );
    }

    #[test]
    fn busy_guards_close_only() {
        assert!(!Caption::Close.enabled(true));
        assert!(Caption::Close.enabled(false));
        assert!(Caption::Minimize.enabled(true));
        assert!(Caption::Maximize.enabled(true));
    }

    #[test]
    fn errors_stay_short_and_unicode_safe() {
        assert_eq!(brief_error("first\nsecond"), "first");
        assert_eq!(brief_error(&"é".repeat(200)).chars().count(), 111);
    }
}

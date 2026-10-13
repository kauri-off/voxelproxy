mod assets;
mod dialogs;
mod log_panel;
mod session_view;
mod setup_view;
mod support;
mod widgets;

use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::StreamExt;
use futures::channel::mpsc::UnboundedReceiver;
use gpui_kit::component::{
    ActiveTheme, Icon, Sizable, StyledExt, Theme, ThemeConfig, ThemeMode, WindowExt,
    badge::Badge,
    button::{Button, ButtonVariants},
    h_flex,
    input::{InputEvent, InputState},
    notification::Notification,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::{
    backend::{self, Backend, UpdateInfo},
    config,
    events::{AppEvent, EventSink, LogLevel, WhichClient},
};

use assets::AppAssets;
use support::{SupportEvent, SupportView};

pub use gpui_kit::assets::IconName;

const SUPPORT_POLL_INTERVAL: Duration = Duration::from_secs(5 * 60);

pub fn run() {
    let (events, rx) = EventSink::channel();
    let backend = Backend::new(events);
    backend.spawn(config::send_startup_ping());

    gpui_kit::application()
        .with_assets(AppAssets)
        .run(move |cx| {
            gpui_kit::init(cx);
            apply_theme(cx);

            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();

            let options = WindowOptions {
                window_bounds: Some(WindowBounds::centered(size(px(980.), px(720.)), cx)),
                window_min_size: Some(size(px(820.), px(600.))),
                titlebar: Some(TitlebarOptions {
                    title: Some("VoxelProxy".into()),
                    ..Default::default()
                }),
                app_id: Some("voxelproxy".into()),
                icon: window_icon(),
                ..Default::default()
            };

            gpui_kit::open_window(options, cx, |window, cx| {
                cx.new(|cx| AppView::new(backend, rx, window, cx))
            })
            .expect("failed to open window");
            cx.activate(true);
        });
}

fn apply_theme(cx: &mut App) {
    let config: ThemeConfig =
        serde_json::from_str(include_str!("theme.json")).expect("invalid theme.json");
    let config = std::rc::Rc::new(config);
    Theme::change(ThemeMode::Dark, None, cx);
    Theme::update(cx, |theme| theme.apply_config(&config));
}

fn window_icon() -> Option<Arc<image::RgbaImage>> {
    let png = include_bytes!("../../icons/icon.png");
    image::load_from_memory(png)
        .ok()
        .map(|img| Arc::new(img.into_rgba8()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Auto,
    Manual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Idle,
    Running,
}

#[derive(Debug, Clone)]
pub enum UpdateState {
    Checking,
    UpToDate,
    CheckFailed(String),
    Available(UpdateInfo),
    Downloading {
        info: UpdateInfo,
        progress: Option<f32>,
    },
    Verifying {
        info: UpdateInfo,
        waited_secs: u64,
    },
    DownloadFailed {
        info: UpdateInfo,
        error: String,
    },
}

impl UpdateState {
    /// The session can't be started until the user deals with the update.
    fn blocks_start(&self) -> bool {
        !matches!(self, UpdateState::UpToDate)
    }
}

pub struct LogEntry {
    pub level: LogLevel,
    pub message: SharedString,
}

#[derive(Default)]
pub struct Clients {
    pub primary: bool,
    pub secondary: bool,
}

impl Clients {
    fn any(&self) -> bool {
        self.primary || self.secondary
    }

    fn both(&self) -> bool {
        self.primary && self.secondary
    }
}

pub struct AppView {
    backend: Arc<Backend>,
    phase: Phase,
    mode: Mode,
    starting: bool,

    server_input: Entity<InputState>,
    port_min_input: Entity<InputState>,
    port_max_input: Entity<InputState>,
    use_windivert: bool,
    elevated: bool,
    show_advanced: bool,
    show_troubleshooting: bool,

    local_ip: SharedString,
    supported_versions: Vec<&'static str>,
    update: UpdateState,

    clients: Clients,
    session_started_at: Option<Instant>,
    nickname: SharedString,
    server_addr: SharedString,
    check_mode: bool,

    logs: Vec<LogEntry>,
    log_open: bool,
    log_scroll: ScrollHandle,

    support: Entity<SupportView>,

    _tasks: Vec<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl AppView {
    fn new(
        backend: Arc<Backend>,
        events: UnboundedReceiver<AppEvent>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let server_input = cx.new(|cx| InputState::new(window, cx).placeholder("mc.funtime.su"));
        let port_min_input = cx.new(|cx| InputState::new(window, cx).default_value("25560"));
        let port_max_input = cx.new(|cx| InputState::new(window, cx).default_value("25570"));

        let subscriptions = vec![cx.subscribe_in(
            &server_input,
            window,
            |this, _, event: &InputEvent, window, cx| {
                if let InputEvent::PressEnter { .. } = event {
                    this.start(window, cx);
                }
            },
        )];

        let support = cx.new(|cx| SupportView::new(backend.clone(), window, cx));
        let mut subscriptions = subscriptions;
        subscriptions.push(cx.observe(&support, |_, _, cx| cx.notify()));
        subscriptions.push(cx.subscribe_in(
            &support,
            window,
            |_, _, event: &SupportEvent, window, cx| {
                let SupportEvent::NewReplies(replies) = event;
                if let Some(last) = replies.last() {
                    window.push_notification(
                        Notification::info(truncate(&last.text, 120))
                            .title("Ответ разработчика")
                            .icon(Icon::new(IconName::MessageCircle)),
                        cx,
                    );
                }
            },
        ));

        let mut this = Self {
            backend,
            phase: Phase::Idle,
            mode: Mode::Auto,
            starting: false,
            server_input,
            port_min_input,
            port_max_input,
            use_windivert: backend::is_windows(),
            elevated: backend::is_elevated(),
            show_advanced: false,
            show_troubleshooting: false,
            local_ip: backend::local_ip().into(),
            supported_versions: backend::supported_versions(),
            update: UpdateState::Checking,
            clients: Clients::default(),
            session_started_at: None,
            nickname: SharedString::default(),
            server_addr: SharedString::default(),
            check_mode: false,
            logs: Vec::new(),
            log_open: false,
            log_scroll: ScrollHandle::new(),
            support,
            _tasks: Vec::new(),
            _subscriptions: subscriptions,
        };

        this._tasks.push(this.pump_events(events, cx));
        this._tasks.push(this.tick_uptime(cx));
        this._tasks.push(this.check_updates(cx));
        this._tasks.push(this.poll_support(cx));
        cx.defer_in(window, |this, window, cx| this.show_changelog(window, cx));
        this
    }

    fn pump_events(
        &self,
        mut events: UnboundedReceiver<AppEvent>,
        cx: &mut Context<Self>,
    ) -> Task<()> {
        cx.spawn(async move |this, cx| {
            while let Some(event) = events.next().await {
                if this
                    .update(cx, |this, cx| this.handle_event(event, cx))
                    .is_err()
                {
                    break;
                }
            }
        })
    }

    fn tick_uptime(&self, cx: &mut Context<Self>) -> Task<()> {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                let alive = this.update(cx, |this, cx| {
                    if this.session_started_at.is_some() {
                        cx.notify();
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        })
    }

    fn check_updates(&self, cx: &mut Context<Self>) -> Task<()> {
        let job = self.backend.check_updates();
        cx.spawn(async move |this, cx| {
            let state = match job.await {
                Ok(Ok(Some(info))) => UpdateState::Available(info),
                Ok(Ok(None)) => UpdateState::UpToDate,
                Ok(Err(e)) => UpdateState::CheckFailed(e),
                Err(e) => UpdateState::CheckFailed(e.to_string()),
            };
            let _ = this.update(cx, |this, cx| {
                if !matches!(this.update, UpdateState::Checking) {
                    return;
                }
                if let UpdateState::CheckFailed(e) = &state {
                    this.push_log(LogLevel::Error, format!("Ошибка проверки обновлений: {e}"));
                }
                this.update = state;
                cx.notify();
            });
        })
    }

    fn poll_support(&self, cx: &mut Context<Self>) -> Task<()> {
        if !config::chat_available() {
            return Task::ready(());
        }
        let support = self.support.clone();
        cx.spawn(async move |_, cx| {
            loop {
                support.update(cx, |support, cx| support.refresh(cx)).await;
                cx.background_executor().timer(SUPPORT_POLL_INTERVAL).await;
            }
        })
    }

    fn handle_event(&mut self, event: AppEvent, cx: &mut Context<Self>) {
        match event {
            AppEvent::Log { level, message } => self.push_log(level, message),
            AppEvent::SessionStarted => {
                self.phase = Phase::Running;
                self.starting = false;
                self.clients = Clients::default();
                self.session_started_at = None;
                self.nickname = SharedString::default();
                self.server_addr = SharedString::default();
                self.check_mode = false;
            }
            AppEvent::SessionEnded => {
                self.phase = Phase::Idle;
                self.starting = false;
                self.clients = Clients::default();
                self.session_started_at = None;
                self.check_mode = false;
            }
            AppEvent::ClientStatus { which, online } => {
                match which {
                    WhichClient::Primary => self.clients.primary = online,
                    WhichClient::Secondary => self.clients.secondary = online,
                }
                if self.clients.both() && self.session_started_at.is_none() {
                    self.session_started_at = Some(Instant::now());
                } else if !self.clients.any() {
                    self.session_started_at = None;
                }
            }
            AppEvent::NickName(name) => self.nickname = name.into(),
            AppEvent::ServerAddr(addr) => self.server_addr = addr.into(),
            AppEvent::UpdateProgress { downloaded, total } => {
                if let UpdateState::Downloading { progress, .. } = &mut self.update {
                    *progress =
                        (total > 0).then(|| (downloaded as f32 / total as f32 * 100.0).min(100.0));
                }
            }
            AppEvent::UpdateVerifying { waited_secs } => {
                if let UpdateState::Downloading { info, .. } | UpdateState::Verifying { info, .. } =
                    &self.update
                {
                    self.update = UpdateState::Verifying {
                        info: info.clone(),
                        waited_secs,
                    };
                }
            }
        }
        cx.notify();
    }

    pub fn push_log(&mut self, level: LogLevel, message: impl Into<SharedString>) {
        if level == LogLevel::Error {
            self.log_open = true;
        }
        self.logs.push(LogEntry {
            level,
            message: message.into(),
        });
        self.log_scroll.scroll_to_bottom();
    }

    fn show_changelog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let entries = backend::pending_changelogs();
        if entries.is_empty() {
            return;
        }
        backend::acknowledge_changelog();
        dialogs::open_changelog(entries, window, cx);
    }

    pub fn set_mode(&mut self, mode: Mode, cx: &mut Context<Self>) {
        self.mode = mode;
        cx.notify();
    }

    pub fn start(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.starting || self.phase == Phase::Running || self.update.blocks_start() {
            return;
        }
        match self.mode {
            Mode::Manual => self.start_manual(window, cx),
            Mode::Auto => self.start_auto(window, cx),
        }
    }

    fn start_manual(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let addr = self.server_input.read(cx).value().trim().to_string();
        if addr.is_empty() {
            self.push_log(LogLevel::Error, "Введите адрес сервера");
            cx.notify();
            return;
        }
        if !validate_server_addr(&addr) {
            self.push_log(
                LogLevel::Error,
                "Неверный формат адреса (пример: mc.funtime.su:25565)",
            );
            cx.notify();
            return;
        }

        if backend::manual_warning_acknowledged() {
            self.launch_manual(addr, cx);
            return;
        }

        let this = cx.entity().downgrade();
        dialogs::open_manual_warning(
            move |window, cx| {
                let _ = this.update(cx, |this, cx| {
                    if let Err(e) = backend::acknowledge_manual_warning() {
                        this.push_log(
                            LogLevel::Warn,
                            format!("Не удалось сохранить подтверждение: {e}"),
                        );
                    }
                    this.launch_manual(addr.clone(), cx);
                });
                window.close_dialog(cx);
            },
            window,
            cx,
        );
    }

    fn launch_manual(&mut self, addr: String, cx: &mut Context<Self>) {
        self.starting = true;
        self.backend.start_manual(addr);
        cx.notify();
    }

    fn start_auto(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (mut port_min, mut port_max) = (25560, 25570);
        if self.use_windivert {
            let parse = |input: &Entity<InputState>| -> Option<u16> {
                input
                    .read(cx)
                    .value()
                    .trim()
                    .parse()
                    .ok()
                    .filter(|p| *p > 0)
            };
            match (parse(&self.port_min_input), parse(&self.port_max_input)) {
                (Some(min), Some(max)) if min <= max => (port_min, port_max) = (min, max),
                _ => {
                    self.show_advanced = true;
                    self.push_log(
                        LogLevel::Error,
                        "Неверный диапазон портов (1–65535, первый ≤ второго)",
                    );
                    cx.notify();
                    return;
                }
            }

            if !self.elevated {
                let this = cx.entity().downgrade();
                dialogs::open_admin_required(
                    move |message, cx| {
                        let _ = this.update(cx, |this, cx| {
                            this.push_log(
                                LogLevel::Error,
                                format!("Не удалось перезапустить от администратора: {message}"),
                            );
                            cx.notify();
                        });
                    },
                    window,
                    cx,
                );
                return;
            }
        }

        self.starting = true;
        self.backend
            .start_auto(self.use_windivert, port_min, port_max);
        cx.notify();
    }

    pub fn stop(&mut self, cx: &mut Context<Self>) {
        self.backend.stop();
        cx.notify();
    }

    pub fn set_check_mode(&mut self, value: bool, cx: &mut Context<Self>) {
        self.check_mode = value;
        self.backend.set_panic_mode(value);
        self.push_log(
            LogLevel::Info,
            if value {
                "Режим проверки включён: новые подключения идут на сервер напрямую"
            } else {
                "Режим проверки выключен"
            },
        );
        cx.notify();
    }

    pub fn install_update(&mut self, cx: &mut Context<Self>) {
        let info = match &self.update {
            UpdateState::Available(info) | UpdateState::DownloadFailed { info, .. } => info.clone(),
            _ => return,
        };
        self.update = UpdateState::Downloading {
            info: info.clone(),
            progress: None,
        };
        cx.notify();

        let job = self
            .backend
            .download_and_install_update(info.link.clone(), info.tag.clone());
        cx.spawn(async move |this, cx| {
            let result = job.await.map_err(|e| e.to_string()).and_then(|r| r);
            let _ = this.update(cx, |this, cx| match result {
                Ok(()) => cx.quit(),
                Err(error) => {
                    this.push_log(
                        LogLevel::Error,
                        format!("Ошибка установки обновления: {error}"),
                    );
                    this.update = UpdateState::DownloadFailed { info, error };
                    cx.notify();
                }
            });
        })
        .detach();
    }

    pub fn open_support(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        support::open(self.support.clone(), window, cx);
    }

    fn render_header(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let running = self.phase == Phase::Running;
        let unread = self.support.read(cx).unread_count();

        h_flex()
            .h(px(52.))
            .px_5()
            .gap_3()
            .border_b_1()
            .border_color(theme.border)
            .bg(theme.title_bar)
            .child(
                h_flex()
                    .id("logo")
                    .gap_2()
                    .cursor_pointer()
                    .on_click(|_, _, _| backend::open_url(backend::REPO_URL))
                    .child(img(assets::APP_ICON).size(px(22.)).rounded(px(5.)))
                    .child(div().font_semibold().child("VoxelProxy")),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(format!("v{}", backend::version())),
            )
            .when(running, |el| el.child(widgets::status_pill(self, cx)))
            .child(div().flex_1())
            .child(
                Badge::new().count(unread).child(
                    Button::new("support")
                        .ghost()
                        .small()
                        .icon(Icon::new(IconName::LifeBuoy))
                        .label("Поддержка")
                        .tooltip("Написать разработчику и получить ответ")
                        .on_click(cx.listener(|this, _, window, cx| this.open_support(window, cx))),
                ),
            )
            .when(running, |el| {
                el.child(
                    Button::new("stop")
                        .danger()
                        .small()
                        .icon(Icon::new(IconName::Square))
                        .label("Остановить")
                        .on_click(cx.listener(|this, _, _, cx| this.stop(cx))),
                )
            })
    }
}

impl Render for AppView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let body = match self.phase {
            Phase::Idle => self.render_setup(window, cx).into_any_element(),
            Phase::Running => self.render_session(window, cx).into_any_element(),
        };

        v_flex()
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .child(self.render_header(cx))
            .child(
                div()
                    .id("body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(
                        v_flex()
                            .w_full()
                            .max_w(px(720.))
                            .mx_auto()
                            .px_6()
                            .py_6()
                            .gap_4()
                            .child(body),
                    ),
            )
            .when(self.phase == Phase::Idle, |el| {
                el.child(self.render_start_bar(cx))
            })
            .child(self.render_log_panel(cx))
    }
}

fn truncate(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((idx, _)) => format!("{}…", &text[..idx]),
        None => text.to_string(),
    }
}

pub fn validate_server_addr(addr: &str) -> bool {
    let addr = addr.trim();
    if addr.is_empty() {
        return false;
    }

    let host = match addr.rsplit_once(':') {
        Some((host, port)) if !host.is_empty() => match port.parse::<u16>() {
            Ok(p) if p > 0 => host,
            _ => return false,
        },
        Some(_) => return false,
        None => addr,
    };

    if host.split('.').count() == 4
        && host
            .split('.')
            .all(|p| p.chars().all(|c| c.is_ascii_digit()))
    {
        return host.parse::<std::net::Ipv4Addr>().is_ok();
    }

    host.split('.').all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    })
}

#[cfg(test)]
mod tests {
    use super::validate_server_addr;

    #[test]
    fn accepts_valid_addresses() {
        for addr in [
            "mc.funtime.su",
            "mc.funtime.su:25565",
            "127.0.0.1",
            "10.0.0.5:25566",
            "localhost",
        ] {
            assert!(validate_server_addr(addr), "{addr}");
        }
    }

    #[test]
    fn rejects_invalid_addresses() {
        for addr in [
            "",
            ":25565",
            "mc.funtime.su:",
            "mc.funtime.su:0",
            "mc.funtime.su:70000",
            "256.1.1.1",
            "-bad.com",
            "a..b",
            "mc fun.su",
        ] {
            assert!(!validate_server_addr(addr), "{addr}");
        }
    }
}

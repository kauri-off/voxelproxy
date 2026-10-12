use gpui_kit::component::{
    ActiveTheme, Disableable, Icon, Sizable, StyledExt,
    alert::Alert,
    button::{Button, ButtonVariants},
    clipboard::Clipboard,
    collapsible::Collapsible,
    h_flex,
    input::Input,
    progress::Progress,
    spinner::Spinner,
    switch::Switch,
    tag::Tag,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::{AppView, IconName, Mode, UpdateState, widgets};
use crate::backend;

impl AppView {
    pub(super) fn render_setup(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        v_flex()
            .w_full()
            .gap_4()
            .children(self.render_update(cx))
            .child(
                v_flex()
                    .gap_1()
                    .child(div().text_xl().font_semibold().child("Как будете играть?"))
                    .child(widgets::hint(
                        "Два клиента Minecraft под одним ником на одном сервере.",
                        cx,
                    )),
            )
            .child(
                h_flex()
                    .gap_3()
                    .items_stretch()
                    .child(self.mode_card(
                        Mode::Auto,
                        IconName::Wifi,
                        "Через раздачу Wi-Fi",
                        "Второе устройство подключено к Wi-Fi, который раздаёт этот ПК.",
                        Tag::success().outline().small().child("Рекомендуется"),
                        cx,
                    ))
                    .child(self.mode_card(
                        Mode::Manual,
                        IconName::Server,
                        "Ручной",
                        "Оба клиента заходят на адрес этого ПК в локальной сети.",
                        Tag::danger().outline().small().child("Детектится"),
                        cx,
                    )),
            )
            .map(|el| match self.mode {
                Mode::Auto => el.child(self.render_auto_setup(cx)),
                Mode::Manual => el.child(self.render_manual_setup(cx)),
            })
    }

    fn mode_card(
        &self,
        mode: Mode,
        icon: IconName,
        title: &'static str,
        description: &'static str,
        tag: Tag,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let selected = self.mode == mode;
        let id = match mode {
            Mode::Auto => "mode-auto",
            Mode::Manual => "mode-manual",
        };

        v_flex()
            .id(id)
            .flex_1()
            .min_w_0()
            .gap_2()
            .p_4()
            .rounded(px(12.))
            .border_2()
            .cursor_pointer()
            .map(|el| {
                if selected {
                    el.border_color(theme.primary)
                        .bg(theme.primary.opacity(0.08))
                } else {
                    el.border_color(theme.border)
                        .hover(|s| s.bg(theme.secondary.opacity(0.5)))
                }
            })
            .on_click(cx.listener(move |this, _, _, cx| this.set_mode(mode, cx)))
            .child(
                h_flex()
                    .gap_2()
                    .child(Icon::new(icon).text_color(if selected {
                        theme.primary
                    } else {
                        theme.muted_foreground
                    }))
                    .child(div().font_semibold().child(title))
                    .child(div().flex_1())
                    .child(tag),
            )
            .child(widgets::hint(description, cx))
    }

    fn render_auto_setup(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let intercept = self.use_windivert && backend::is_windows();
        v_flex()
            .gap_3()
            .map(|el| {
                if intercept {
                    el.child(self.render_hotspot_checklist(cx))
                        .child(self.render_troubleshooting(cx))
                } else {
                    el.child(self.render_no_intercept_info(cx))
                }
            })
            .when(backend::is_windows(), |el| {
                el.child(self.render_advanced(cx))
            })
    }

    fn render_hotspot_checklist(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let elevated = self.elevated;
        widgets::card(cx)
            .child(widgets::section_title("Перед запуском", cx))
            .child(widgets::numbered_step(
                1,
                false,
                "Включите раздачу Wi-Fi на этом ПК: Параметры → Сеть и Интернет → Мобильный хот-спот.",
                cx,
            ))
            .child(widgets::numbered_step(
                2,
                false,
                "Подключите второе устройство (телефон, ноутбук) к Wi-Fi, который раздаёт этот ПК, а не к домашнему роутеру.",
                cx,
            ))
            .child(
                h_flex()
                    .gap_3()
                    .justify_between()
                    .child(widgets::numbered_step(
                        3,
                        elevated,
                        if elevated {
                            "VoxelProxy запущен от имени администратора."
                        } else {
                            "Запустите VoxelProxy от имени администратора — без этого трафик не прочитать."
                        },
                        cx,
                    ))
                    .when(!elevated, |el| {
                        el.child(
                            Button::new("relaunch-admin")
                                .small()
                                .outline()
                                .icon(Icon::new(IconName::ShieldCheck))
                                .label("Перезапустить")
                                .on_click(cx.listener(|_, _, _, cx| {
                                    let this = cx.entity().downgrade();
                                    super::dialogs::relaunch_as_admin(
                                        move |e, cx| {
                                            let _ = this.update(cx, |this, cx| {
                                                this.push_log(
                                                    crate::events::LogLevel::Error,
                                                    format!("Не удалось перезапустить от администратора: {e}"),
                                                );
                                                cx.notify();
                                            });
                                        },
                                        cx,
                                    );
                                })),
                        )
                    }),
            )
            .child(widgets::numbered_step(
                4,
                false,
                "Нажмите «Запустить» — дальше приложение покажет, куда заходить с каждого клиента.",
                cx,
            ))
            .child(widgets::hint(
                "Зачем всё это: VoxelProxy видит игру второго устройства, только если его интернет идёт через этот ПК. Через домашний роутер авто-режим не работает.",
                cx,
            ))
    }

    fn render_no_intercept_info(&self, cx: &mut Context<Self>) -> impl IntoElement {
        widgets::card(cx)
            .child(widgets::section_title("Без перехвата трафика", cx))
            .child(widgets::hint(
                "Перехват выключен. Направьте трафик второго устройства на порт 25565 этого ПК самостоятельно (например, через iptables, рутованный телефон или свой DNS). Затем зайдите на сервер сначала со второго устройства, потом основным клиентом на 127.0.0.1:25565.",
                cx,
            ))
    }

    fn render_troubleshooting(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let open = self.show_troubleshooting;
        let toggle = self.disclosure(
            "troubleshooting-toggle",
            IconName::LifeBuoy,
            "Если что-то пошло не так",
            open,
            |this| &mut this.show_troubleshooting,
            cx,
        );
        let theme = cx.theme().clone();
        let code = |id: &'static str, text: &'static str| {
            h_flex()
                .gap_2()
                .items_start()
                .justify_between()
                .px_3()
                .py_2()
                .rounded(px(6.))
                .bg(theme.background)
                .border_1()
                .border_color(theme.border)
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .font_family(theme.mono_font_family.clone())
                        .text_sm()
                        .child(text),
                )
                .child(Clipboard::new(id).value(text).tooltip("Скопировать"))
        };
        let item = |title: &'static str, body: &'static str| {
            v_flex()
                .gap_1()
                .child(div().text_sm().font_semibold().child(title))
                .child(widgets::hint(body, cx))
        };

        Collapsible::new()
            .open(open)
            .motion_id("troubleshooting")
            .w_full()
            .child(toggle)
            .content(
                v_flex()
                    .gap_4()
                    .pt_3()
                    .px_1()
                    .child(
                        v_flex()
                            .gap_2()
                            .child(item(
                                "Клиент не подключается или на втором устройстве пропал интернет",
                                "Завис перехват трафика (WinDivert) или сама раздача Wi-Fi. Выполните в PowerShell от администратора, затем снова включите раздачу и перезапустите VoxelProxy:",
                            ))
                            .child(code("troubleshooting-copy", "sc.exe stop windivert\nStop-Service SharedAccess")),
                    )
                    .child(item(
                        "На втором устройстве не открываются сайты",
                        "Пропишите DNS вручную: в настройках Wi-Fi (IPv4) укажите 1.1.1.1 и 8.8.8.8 и переподключитесь.",
                    ))
                    .child(item(
                        "Включён VPN в TUN-режиме",
                        "TUN-адаптер забирает трафик раньше WinDivert. Отключите VPN на этом ПК.",
                    ))
                    .child(item(
                        "Лицензионный (online-mode) сервер",
                        "Не поддерживается напрямую. Поставьте перед VoxelProxy ViaProxy с настроенным аккаунтом.",
                    )),
            )
    }

    fn render_advanced(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let open = self.show_advanced;
        let blocked = self.update.blocks_start();

        Collapsible::new()
            .open(open)
            .motion_id("advanced")
            .w_full()
            .child(self.disclosure(
                "advanced-toggle",
                IconName::Settings,
                "Дополнительные настройки",
                open,
                |this| &mut this.show_advanced,
                cx,
            ))
            .content(
                v_flex()
                    .gap_3()
                    .pt_3()
                    .px_1()
                    .child(
                        h_flex()
                            .gap_3()
                            .justify_between()
                            .child(
                                v_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .gap_0p5()
                                    .child(div().text_sm().font_semibold().child(
                                        "Перехватывать трафик раздачи Wi-Fi",
                                    ))
                                    .child(widgets::hint(
                                        "Не выключайте, если не программист. Без перехвата трафик второго устройства нужно направлять на этот ПК самостоятельно.",
                                        cx,
                                    )),
                            )
                            .child(
                                div().flex_shrink_0().child(
                                Switch::new("use-windivert")
                                    .checked(self.use_windivert)
                                    .disabled(blocked)
                                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                        this.use_windivert = *checked;
                                        cx.notify();
                                    })),
                                ),
                            ),
                    )
                    .when(self.use_windivert, |el| {
                        el.child(
                            h_flex()
                                .gap_3()
                                .justify_between()
                                .child(
                                    v_flex()
                                        .flex_1()
                                        .min_w_0()
                                        .gap_0p5()
                                        .child(div().text_sm().font_semibold().child("Порты серверов"))
                                        .child(widgets::hint(
                                            "Какие порты перехватывать у второго устройства.",
                                            cx,
                                        )),
                                )
                                .child(
                                    h_flex()
                                        .flex_shrink_0()
                                        .gap_2()
                                        .child(
                                            div().w(px(96.)).child(
                                                Input::new(&self.port_min_input)
                                                    .small()
                                                    .disabled(blocked),
                                            ),
                                        )
                                        .child("—")
                                        .child(
                                            div().w(px(96.)).child(
                                                Input::new(&self.port_max_input)
                                                    .small()
                                                    .disabled(blocked),
                                            ),
                                        ),
                                ),
                        )
                    }),
            )
    }

    fn disclosure(
        &self,
        id: &'static str,
        icon: IconName,
        label: &'static str,
        open: bool,
        field: fn(&mut AppView) -> &mut bool,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let theme = cx.theme();
        h_flex()
            .id(id)
            .gap_2()
            .py_1()
            .cursor_pointer()
            .text_sm()
            .text_color(theme.muted_foreground)
            .hover(|s| s.text_color(theme.foreground))
            .on_click(cx.listener(move |this, _, _, cx| {
                let flag = field(this);
                *flag = !*flag;
                cx.notify();
            }))
            .child(Icon::new(if open {
                IconName::ChevronDown
            } else {
                IconName::ChevronRight
            }))
            .child(Icon::new(icon))
            .child(label)
    }

    fn render_manual_setup(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        widgets::card(cx)
            .child(widgets::section_title("Сервер", cx))
            .child(
                Input::new(&self.server_input)
                    .prefix(Icon::new(IconName::Server).text_color(theme.muted_foreground))
                    .disabled(self.update.blocks_start()),
            )
            .child(widgets::hint(
                "Адрес с портом или без, например mc.funtime.su или 1.2.3.4:25565. После запуска приложение покажет, куда подключать клиенты.",
                cx,
            ))
            .child(
                Alert::warning(
                    "manual-warning",
                    "Этот режим видно серверу — за него могут забанить. Для обычной игры используйте «Через раздачу Wi-Fi».",
                )
                .small(),
            )
    }

    pub(super) fn render_start_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let blocked = self.update.blocks_start();
        let theme = cx.theme();
        h_flex()
            .w_full()
            .border_t_1()
            .border_color(theme.border)
            .child(
                h_flex()
                    .w_full()
                    .max_w(px(720.))
                    .mx_auto()
                    .px_6()
                    .py_3()
                    .gap_3()
                    .justify_between()
                    .child(
                        h_flex()
                            .gap_1()
                            .flex_wrap()
                            .child(widgets::hint("Версии:", cx))
                            .children(
                                self.supported_versions
                                    .iter()
                                    .map(|v| Tag::secondary().small().child(*v)),
                            ),
                    )
                    .child(
                        Button::new("start")
                            .primary()
                            .large()
                            .icon(Icon::new(IconName::Play))
                            .label(if self.starting {
                                "Запуск…"
                            } else {
                                "Запустить"
                            })
                            .loading(self.starting)
                            .disabled(blocked || self.starting)
                            .on_click(cx.listener(|this, _, window, cx| this.start(window, cx))),
                    ),
            )
    }

    fn render_update(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let theme = cx.theme();
        let el = match &self.update {
            UpdateState::UpToDate => return None,
            UpdateState::Checking => h_flex()
                .gap_2()
                .text_sm()
                .text_color(theme.muted_foreground)
                .child(Spinner::new().small())
                .child("Проверка обновлений…")
                .into_any_element(),
            UpdateState::Available(info) => widgets::card(cx)
                .border_color(theme.primary)
                .child(
                    h_flex()
                        .gap_3()
                        .justify_between()
                        .child(
                            v_flex()
                                .gap_0p5()
                                .child(
                                    div()
                                        .font_semibold()
                                        .child(format!("Доступна новая версия {}", info.tag)),
                                )
                                .child(widgets::hint(
                                    "Обновите приложение, чтобы продолжить. Это займёт меньше минуты.",
                                    cx,
                                )),
                        )
                        .child(
                            Button::new("update")
                                .primary()
                                .icon(Icon::new(IconName::Download))
                                .label("Обновить")
                                .on_click(cx.listener(|this, _, _, cx| this.install_update(cx))),
                        ),
                )
                .into_any_element(),
            UpdateState::Downloading { info, progress } => widgets::card(cx)
                .child(
                    h_flex()
                        .justify_between()
                        .text_sm()
                        .child(format!("Загрузка {}", info.tag))
                        .child(match progress {
                            Some(p) => format!("{p:.0}%"),
                            None => "…".to_string(),
                        }),
                )
                .child(
                    Progress::new("update-progress")
                        .loading(progress.is_none())
                        .value(progress.unwrap_or(0.0)),
                )
                .into_any_element(),
            UpdateState::DownloadFailed { error, .. } => widgets::card(cx)
                .border_color(theme.danger)
                .child(
                    h_flex()
                        .gap_3()
                        .justify_between()
                        .child(
                            v_flex()
                                .gap_0p5()
                                .child(div().font_semibold().child("Не удалось скачать обновление"))
                                .child(widgets::hint(error.clone(), cx)),
                        )
                        .child(
                            Button::new("update-retry")
                                .icon(Icon::new(IconName::RefreshCw))
                                .label("Повторить")
                                .on_click(cx.listener(|this, _, _, cx| this.install_update(cx))),
                        ),
                )
                .into_any_element(),
            UpdateState::CheckFailed(_) => widgets::card(cx)
                .border_color(theme.danger)
                .child(
                    h_flex()
                        .gap_3()
                        .justify_between()
                        .child(
                            v_flex()
                                .gap_0p5()
                                .child(div().font_semibold().child("Не удалось проверить обновления"))
                                .child(widgets::hint(
                                    "Проверьте интернет. Если не помогает — скачайте свежую версию со страницы релизов.",
                                    cx,
                                )),
                        )
                        .child(
                            h_flex()
                                .gap_2()
                                .child(
                                    Button::new("update-recheck")
                                        .icon(Icon::new(IconName::RefreshCw))
                                        .label("Повторить")
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.update = UpdateState::Checking;
                                            let task = this.check_updates(cx);
                                            this._tasks.push(task);
                                            cx.notify();
                                        })),
                                )
                                .child(
                                    Button::new("update-releases")
                                        .ghost()
                                        .icon(Icon::new(IconName::ExternalLink))
                                        .label("Релизы")
                                        .on_click(|_, _, _| backend::open_url(backend::RELEASES_URL)),
                                ),
                        ),
                )
                .into_any_element(),
        };
        Some(el)
    }
}

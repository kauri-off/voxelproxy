use std::time::Duration;

use gpui_kit::component::{ActiveTheme, Icon, Sizable, StyledExt, h_flex, switch::Switch, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::{AppView, IconName, Mode, widgets};
use crate::backend;

const PROXY_ADDR: &str = "127.0.0.1:25565";

impl AppView {
    pub(super) fn render_session(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let live = self.session_started_at.is_some();
        let intercept = self.mode == Mode::Auto && self.use_windivert && backend::is_windows();

        v_flex()
            .w_full()
            .gap_4()
            .map(|el| {
                if live {
                    el.child(self.render_live_session(cx))
                } else if intercept {
                    el.child(self.render_hotspot_steps(cx))
                } else {
                    el.child(self.render_waiting(cx))
                }
            })
            .when(self.mode == Mode::Auto, |el| {
                el.child(self.render_check_mode(cx))
            })
    }

    fn client_labels(&self) -> (&'static str, &'static str) {
        match self.mode {
            Mode::Auto => ("Этот ПК", "Второе устройство"),
            Mode::Manual => ("Основной клиент", "Второй клиент"),
        }
    }

    fn render_clients(&self, cx: &App) -> impl IntoElement {
        let (primary, secondary) = self.client_labels();
        let row = |label: &'static str, online: bool| {
            h_flex()
                .gap_2()
                .text_sm()
                .child(widgets::status_dot(online, cx))
                .child(label)
                .child(
                    div()
                        .text_color(cx.theme().muted_foreground)
                        .child(if online { "в игре" } else { "ждём" }),
                )
        };
        h_flex()
            .gap_6()
            .child(row(primary, self.clients.primary))
            .child(row(secondary, self.clients.secondary))
    }

    fn render_hotspot_steps(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let secondary = self.clients.secondary;
        let primary = self.clients.primary;
        widgets::card(cx)
            .gap_4()
            .child(
                v_flex()
                    .gap_1()
                    .child(div().text_lg().font_semibold().child("Подключите клиенты по порядку"))
                    .child(widgets::hint("Порядок важен: сначала второе устройство, потом этот ПК.", cx)),
            )
            .child(
                v_flex()
                    .gap_2()
                    .child(widgets::numbered_step(
                        1,
                        secondary,
                        "На втором устройстве зайдите на сервер как обычно, например mc.funtime.su.",
                        cx,
                    ))
                    .when(secondary, |el| {
                        el.child(div().pl(px(36.)).child(widgets::hint("Второе устройство подключено.", cx)))
                    }),
            )
            .child(
                v_flex()
                    .gap_2()
                    .child(widgets::numbered_step(
                        2,
                        primary,
                        "На этом ПК в Minecraft добавьте сервер с адресом:",
                        cx,
                    ))
                    .child(div().pl(px(36.)).child(widgets::copyable_address("copy-local", PROXY_ADDR, cx))),
            )
            .child(widgets::numbered_step(
                3,
                false,
                "Готово — играйте с этого ПК. Если он выйдет из игры, управление перейдёт второму устройству.",
                cx,
            ))
    }

    fn render_waiting(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let addr = match self.mode {
            Mode::Auto => PROXY_ADDR.to_string(),
            Mode::Manual => format!("{}:25565", self.local_ip),
        };
        let note = match self.mode {
            Mode::Auto => {
                "Сначала зайдите на сервер со второго устройства, затем основным клиентом на этот адрес."
            }
            Mode::Manual => {
                "Подключите оба клиента к этому адресу. Первый подключившийся станет основным."
            }
        };
        widgets::card(cx)
            .gap_4()
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_lg()
                            .font_semibold()
                            .child(if self.clients.any() {
                                "Ждём второй клиент"
                            } else {
                                "Ждём клиентов"
                            }),
                    )
                    .child(widgets::hint(note, cx)),
            )
            .child(widgets::copyable_address("copy-addr", addr, cx))
            .child(self.render_clients(cx))
    }

    fn render_live_session(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let uptime = self
            .session_started_at
            .map(|t| format_uptime(t.elapsed()))
            .unwrap_or_default();
        let nickname = if self.nickname.is_empty() {
            SharedString::from("…")
        } else {
            self.nickname.clone()
        };

        widgets::card(cx)
            .gap_4()
            .border_color(theme.success.opacity(0.5))
            .child(
                h_flex()
                    .gap_3()
                    .child(
                        div()
                            .size(px(44.))
                            .rounded(px(10.))
                            .bg(theme.success.opacity(0.15))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(Icon::new(IconName::Gamepad2).text_color(theme.success)),
                    )
                    .child(
                        v_flex()
                            .gap_0p5()
                            .child(div().text_xl().font_semibold().child(nickname))
                            .when(!self.server_addr.is_empty(), |el| {
                                el.child(widgets::hint(format!("на {}", self.server_addr), cx))
                            }),
                    )
                    .child(div().flex_1())
                    .child(
                        h_flex()
                            .gap_1p5()
                            .text_color(theme.muted_foreground)
                            .child(Icon::new(IconName::Clock).small())
                            .child(
                                div()
                                    .font_family(theme.mono_font_family.clone())
                                    .text_lg()
                                    .text_color(theme.foreground)
                                    .child(uptime),
                            ),
                    ),
            )
            .child(self.render_clients(cx))
    }

    fn render_check_mode(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let on = self.check_mode;

        widgets::card(cx)
            .when(on, |el| {
                el.border_color(theme.warning)
                    .bg(theme.warning.opacity(0.1))
            })
            .child(
                h_flex()
                    .gap_3()
                    .child(
                        Icon::new(if on { IconName::ShieldAlert } else { IconName::ShieldCheck })
                            .text_color(if on { theme.warning } else { theme.muted_foreground }),
                    )
                    .child(
                        div()
                            .flex_1()
                            .font_semibold()
                            .child(if on {
                                "Режим проверки включён"
                            } else {
                                "Вызвали на проверку?"
                            }),
                    )
                    .child(
                        Switch::new("check-mode")
                            .checked(on)
                            .color(theme.warning)
                            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                this.set_check_mode(*checked, cx)
                            })),
                    ),
            )
            .child(widgets::hint(
                if on {
                    "Каждое новое подключение сейчас идёт на сервер напрямую — как обычная игра, без второго клиента. Текущая сессия продолжается. Выключите после проверки."
                } else {
                    "Новые подключения пойдут на сервер напрямую, как обычная игра без второго клиента, а текущая сессия не прервётся."
                },
                cx,
            ))
    }
}

fn format_uptime(elapsed: Duration) -> String {
    let s = elapsed.as_secs();
    let (h, m, s) = (s / 3600, (s % 3600) / 60, s % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::format_uptime;
    use std::time::Duration;

    #[test]
    fn uptime_format() {
        assert_eq!(format_uptime(Duration::from_secs(0)), "00:00");
        assert_eq!(format_uptime(Duration::from_secs(75)), "01:15");
        assert_eq!(format_uptime(Duration::from_secs(3661)), "1:01:01");
    }
}

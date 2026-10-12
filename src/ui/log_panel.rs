use gpui_kit::component::{
    ActiveTheme, Icon, Sizable,
    button::{Button, ButtonVariants},
    h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::base::SelectableText;
use gpui_kit::*;

use super::{AppView, IconName};
use crate::events::LogLevel;

impl AppView {
    pub(super) fn render_log_panel(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let open = self.log_open;
        let errors = self
            .logs
            .iter()
            .filter(|l| l.level == LogLevel::Error)
            .count();

        v_flex()
            .w_full()
            .border_t_1()
            .border_color(theme.border)
            .bg(theme.title_bar)
            .child(
                h_flex()
                    .id("log-header")
                    .h(px(36.))
                    .px_5()
                    .gap_2()
                    .cursor_pointer()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.log_open = !this.log_open;
                        cx.notify();
                    }))
                    .child(Icon::new(IconName::SquareTerminal).small())
                    .child("Журнал")
                    .child(format!("({})", self.logs.len()))
                    .when(errors > 0, |el| {
                        el.child(
                            div()
                                .text_xs()
                                .text_color(theme.danger)
                                .child(format!("ошибок: {errors}")),
                        )
                    })
                    .child(div().flex_1())
                    .when(open && !self.logs.is_empty(), |el| {
                        el.child(
                            Button::new("log-copy")
                                .ghost()
                                .xsmall()
                                .label("Копировать")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    cx.stop_propagation();
                                    let text = this
                                        .logs
                                        .iter()
                                        .map(|l| l.message.as_ref())
                                        .collect::<Vec<_>>()
                                        .join("\n");
                                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                                })),
                        )
                        .child(
                            Button::new("log-clear")
                                .ghost()
                                .xsmall()
                                .label("Очистить")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.logs.clear();
                                    cx.notify();
                                })),
                        )
                    })
                    .child(
                        Icon::new(if open {
                            IconName::ChevronDown
                        } else {
                            IconName::ChevronUp
                        })
                        .small(),
                    ),
            )
            .when(open, |el| {
                el.child(
                    div()
                        .id("log-body")
                        .h(px(180.))
                        .px_5()
                        .pb_3()
                        .overflow_y_scroll()
                        .track_scroll(&self.log_scroll)
                        .font_family(theme.mono_font_family.clone())
                        .text_xs()
                        .when(self.logs.is_empty(), |el| {
                            el.child(div().text_color(theme.muted_foreground).child("Пока пусто"))
                        })
                        .children(self.logs.iter().enumerate().map(|(ix, entry)| {
                            let color = match entry.level {
                                LogLevel::Info => theme.foreground.opacity(0.8),
                                LogLevel::Success => theme.success,
                                LogLevel::Warn => theme.warning,
                                LogLevel::Error => theme.danger,
                            };
                            div()
                                .py_0p5()
                                .text_color(color)
                                .child(
                                    SelectableText::new(("log-line", ix), entry.message.clone())
                                        .document_order(ix as u64),
                                )
                        })),
                )
            })
    }
}

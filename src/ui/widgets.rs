use gpui_kit::component::{
    ActiveTheme, Icon, Sizable, StyledExt, clipboard::Clipboard, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::{AppView, IconName};

pub fn card(cx: &App) -> Div {
    let theme = cx.theme();
    v_flex()
        .w_full()
        .gap_3()
        .p_5()
        .rounded(px(12.))
        .border_1()
        .border_color(theme.border)
        .bg(theme.secondary.opacity(0.35))
}

pub fn section_title(title: impl Into<SharedString>, cx: &App) -> Div {
    div()
        .text_xs()
        .font_semibold()
        .text_color(cx.theme().muted_foreground)
        .child(title.into().to_uppercase())
}

pub fn hint(text: impl Into<SharedString>, cx: &App) -> Div {
    div()
        .text_sm()
        .text_color(cx.theme().muted_foreground)
        .child(text.into())
}

pub fn status_dot(online: bool, cx: &App) -> Div {
    let theme = cx.theme();
    div()
        .size(px(9.))
        .flex_shrink_0()
        .rounded_full()
        .bg(if online {
            theme.success
        } else {
            theme.muted_foreground.opacity(0.5)
        })
}

pub fn copyable_address(id: &'static str, addr: impl Into<SharedString>, cx: &App) -> Div {
    let addr = addr.into();
    let theme = cx.theme();
    h_flex()
        .gap_2()
        .px_3()
        .py_1p5()
        .rounded(px(8.))
        .bg(theme.background)
        .border_1()
        .border_color(theme.border)
        .child(
            div()
                .font_family(theme.mono_font_family.clone())
                .text_base()
                .font_semibold()
                .child(addr.clone()),
        )
        .child(Clipboard::new(id).value(addr).tooltip("Скопировать"))
}

pub fn numbered_step(n: usize, done: bool, title: impl Into<SharedString>, cx: &App) -> Div {
    let theme = cx.theme();
    h_flex()
        .flex_1()
        .min_w_0()
        .gap_3()
        .items_start()
        .child(
            div()
                .size(px(24.))
                .flex_shrink_0()
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .text_xs()
                .font_semibold()
                .map(|el| {
                    if done {
                        el.bg(theme.success)
                            .text_color(theme.success_foreground)
                            .child(Icon::new(IconName::Check).xsmall())
                    } else {
                        el.bg(theme.secondary)
                            .border_1()
                            .border_color(theme.border)
                            .child(n.to_string())
                    }
                }),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .pt(px(2.))
                .text_sm()
                .when(done, |el| el.text_color(theme.muted_foreground))
                .child(title.into()),
        )
}

pub fn status_pill(app: &AppView, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    let (color, label) = if app.check_mode {
        (theme.warning, "Режим проверки")
    } else if app.session_started_at.is_some() {
        (theme.success, "Сессия активна")
    } else {
        (theme.info, "Ожидание клиентов")
    };
    h_flex()
        .gap_1p5()
        .px_2()
        .py_0p5()
        .rounded_full()
        .bg(color.opacity(0.15))
        .text_xs()
        .text_color(color)
        .child(div().size(px(6.)).rounded_full().bg(color))
        .child(label)
}

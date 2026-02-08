use crate::ui::{
    app::MainWindow,
    components::{div::flex_row, icons::icon::icon},
    variables::Variables,
    views::AppView,
};
use gpui::prelude::FluentBuilder;
use gpui::*;

#[derive(IntoElement)]
pub struct NavButton {
    icon: SharedString,
    label: Option<SharedString>,
    count: Option<usize>,
    target_view: AppView,
}

impl NavButton {
    pub fn new(
        icon: impl Into<SharedString>,
        label: Option<&str>,
        count: Option<usize>,
        target_view: AppView,
    ) -> Self {
        Self {
            icon: icon.into(),
            label: label.map(|s| SharedString::from(s.to_string())),
            count: count,
            target_view,
        }
    }
}

impl RenderOnce for NavButton {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let variables = cx.global::<Variables>();
        let is_active = _window
            .root::<MainWindow>()
            .and_then(|root| root.map(|root| root.read(cx).current_view() == self.target_view))
            .unwrap_or(false);
        let target_view = self.target_view;
        let icon_path = self.icon;
        let label = self.label;
        let count = self.count;

        let (default_color, hover_color) = if is_active {
            (variables.text, variables.text)
        } else {
            (variables.text_secondary, variables.text)
        };

        let group_name = if let Some(ref txt) = label {
            format!("nav_btn_{}", txt)
        } else {
            "nav_btn_icon_only".to_string()
        };

        flex_row()
            .group(group_name.clone())
            .id(group_name.clone())
            .justify_between()
            .cursor_pointer()
            .text_color(default_color)
            .group_hover(group_name.clone(), |s| s.text_color(hover_color))
            .child(
                flex_row()
                    .gap(px(variables.padding_8))
                    .child(
                        icon(icon_path)
                            .text_color(default_color)
                            .group_hover(group_name.clone(), |s| s.text_color(hover_color)),
                    )
                    .when_some(label, |this, label_text| this.child(label_text)),
            )
            .when_some(count, |this, count_val| {
                if count_val != 0 {
                    this.child(count_val.to_string())
                } else {
                    this
                }
            })
            .on_mouse_down(MouseButton::Left, move |_event, window, cx| {
                if let Some(Some(root)) = window.root::<MainWindow>() {
                    root.update(cx, |view, cx| {
                        view.set_current_view(target_view, window, cx);
                    });
                }
            })
    }
}

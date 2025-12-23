use crate::{
    data::state::State,
    ui::{
        components::{div::flex_row, icons::icon::icon},
        variables::Variables,
        views::AppView,
    },
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
        let state = cx.global::<State>();
        let current_view = state.get_current_view_sync();
        let is_active = current_view == self.target_view;
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
            .child(
                flex_row()
                    .gap(px(variables.padding_8))
                    .child(
                        icon(icon_path)
                            .text_color(default_color)
                            .group_hover(group_name.clone(), |s| s.text_color(hover_color)),
                    )
                    .when_some(label, |this, label_text| {
                        this.child(label_text.clone())
                            .text_color(default_color)
                            .group_hover(group_name.clone(), |s| s.text_color(hover_color))
                    }),
            )
            .when_some(count, |this, count| {
                if count != 0 {
                    this.child(count.to_string())
                        .text_color(variables.text)
                } else {
                    this
                }
            })
            .on_mouse_down(MouseButton::Left, move |_event, window, cx| {
                let state = cx.global::<State>().clone();
                state.set_current_view_sync(target_view);
                window.refresh();
            })
    }
}

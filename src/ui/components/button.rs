use gpui::{prelude::FluentBuilder as _, *};
use std::rc::Rc;

use crate::ui::{
    components::{div::flex_row, icons::icon},
    variables::Variables,
};

pub type ClickHandler = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ButtonVariant {
    Default,
    Active,
    Accent,
}

impl ButtonVariant {
    fn colors(self, variables: &Variables) -> (Rgba, Rgba, Rgba) {
        match self {
            Self::Default => (variables.element, variables.element_hover, variables.text),
            Self::Active => (variables.text, variables.text_hover, variables.background),
            Self::Accent => (variables.accent, variables.accent_hover, variables.text),
        }
    }
}

#[derive(IntoElement)]
pub struct Button {
    id: ElementId,
    group_id: SharedString,
    base: Stateful<Div>,
    children: Vec<AnyElement>,
    on_click: Option<ClickHandler>,
    icon: Option<SharedString>,
    color: Option<Rgba>,
    hover_color: Option<Rgba>,
    bg_color: Option<Rgba>,
    variant: Option<ButtonVariant>,
}

impl Button {
    pub fn new(id: impl Into<SharedString>) -> Self {
        let id: SharedString = id.into();
        let element_id: ElementId = id.clone().into();
        let group_id = SharedString::from(format!("btn_{}", id));

        Self {
            id: element_id.clone(),
            group_id,
            base: flex_row().id(element_id),
            children: Vec::new(),
            on_click: None,
            icon: None,
            color: None,
            hover_color: None,
            bg_color: None,
            variant: None,
        }
    }

    pub fn variant(mut self, variant: ButtonVariant) -> Self {
        self.variant = Some(variant);
        self
    }

    pub fn icon(mut self, icon_path: impl Into<SharedString>) -> Self {
        self.icon = Some(icon_path.into());
        self
    }

    pub fn color(mut self, color: Rgba) -> Self {
        self.color = Some(color);
        self
    }

    pub fn hover_color(mut self, color: Rgba) -> Self {
        self.hover_color = Some(color);
        self
    }

    pub fn bg_color(mut self, color: Rgba) -> Self {
        self.bg_color = Some(color);
        self
    }

    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Rc::new(handler));
        self
    }
}

impl Styled for Button {
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.style()
    }
}

impl ParentElement for Button {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements)
    }
}

impl InteractiveElement for Button {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.base.interactivity()
    }
}

impl RenderOnce for Button {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let variables = cx.global::<Variables>();
        let filled = self.variant.map(|variant| variant.colors(variables));
        let color = match filled {
            Some((_, _, text)) => text,
            None => self.color.unwrap_or(variables.text_secondary),
        };
        let hover_color = match filled {
            Some((_, _, text)) => text,
            None => self.hover_color.unwrap_or(variables.text),
        };
        let group_id = self.group_id.clone();

        let icon_element = self.icon.map(|icon_path| {
            icon(icon_path)
                .text_color(color)
                .group_hover(group_id.clone(), |s| s.text_color(hover_color))
        });

        self.base
            .id(self.id)
            .cursor_pointer()
            .group(self.group_id.clone())
            .text_color(color)
            .map(|this| match filled {
                Some((bg, hover_bg, _)) => this
                    .px(px(16.0))
                    .py(px(8.0))
                    .gap(px(8.0))
                    .bg(bg)
                    .hover(move |s| s.bg(hover_bg))
                    .when(self.variant == Some(ButtonVariant::Active), |this| {
                        this.font_weight(FontWeight::MEDIUM)
                    }),
                None => this
                    .p(px(8.0))
                    .when_some(self.bg_color, |this, bg| this.bg(bg)),
            })
            .group_hover(self.group_id, |s| s.text_color(hover_color))
            .when_some(self.on_click, |this, on_click| {
                this.on_click(move |event, window, cx| {
                    on_click(event, window, cx);
                })
            })
            .when_some(icon_element, |this, icon_el| this.child(icon_el))
            .children(self.children)
    }
}

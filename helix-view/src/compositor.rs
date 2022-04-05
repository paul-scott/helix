// Each component declares it's own size constraints and gets fitted based on it's parent.
// Q: how does this work with popups?
// cursive does compositor.screen_mut().add_layer_at(pos::absolute(x, y), <component>)
use crate::graphics::{CursorKind, Rect};
use helix_core::Position;

pub type Callback = Box<dyn FnOnce(&mut Compositor, &mut Context)>;

// Cursive-inspired
pub enum EventResult {
    Ignored(Option<Callback>),
    Consumed(Option<Callback>),
}

use crate::job::Jobs;
use crate::Editor;

pub use crate::input::Event;

pub struct Context<'a> {
    pub editor: &'a mut Editor,
    pub jobs: &'a mut Jobs,
}

#[cfg(feature = "term")]
pub mod term {
    use super::*;
    pub use tui::buffer::Buffer as Surface;

    pub struct RenderContext<'a> {
        pub editor: &'a Editor,
        pub surface: &'a mut Surface,
        pub scroll: Option<usize>,
    }

    pub trait Render {
        /// Render the component onto the provided surface.
        fn render(&mut self, area: Rect, ctx: &mut RenderContext);

        // TODO: make required_size be layout() instead and part of this trait?

        /// Get cursor position and cursor kind.
        fn cursor(&self, _area: Rect, _ctx: &Editor) -> (Option<Position>, CursorKind) {
            (None, CursorKind::Hidden)
        }
    }
}

#[cfg(feature = "ui")]
pub mod ui {
    use super::*;
    pub type Surface = ();

    pub struct RenderContext<'a> {
        pub editor: &'a Editor,
        // pub surface: &'a mut Surface,
        pub scroll: Option<usize>,
    }

    pub trait Render {
        /// Render the component onto the provided surface.
        fn render(&mut self, area: Rect, ctx: &mut RenderContext) {
            // TODO:
        }

        // TODO: make required_size be layout() instead and part of this trait?
    }
}

#[cfg(feature = "term")]
pub use term::*;

#[cfg(feature = "ui")]
pub use ui::*;

pub trait Component: Any + AnyComponent + Render {
    /// Process input events, return true if handled.
    fn handle_event(&mut self, _event: Event, _ctx: &mut Context) -> EventResult {
        EventResult::Ignored(None)
    }

    /// Should redraw? Useful for saving redraw cycles if we know component didn't change.
    fn should_update(&self) -> bool {
        true
    }

    /// May be used by the parent component to compute the child area.
    /// viewport is the maximum allowed area, and the child should stay within those bounds.
    ///
    /// The returned size might be larger than the viewport if the child is too big to fit.
    /// In this case the parent can use the values to calculate scroll.
    fn required_size(&mut self, _viewport: (u16, u16)) -> Option<(u16, u16)> {
        None
    }

    fn type_name(&self) -> &'static str {
        std::any::type_name::<Self>()
    }

    fn id(&self) -> Option<&'static str> {
        None
    }
}

pub struct Compositor {
    layers: Vec<Box<dyn Component>>,
    area: Rect,

    // TODO: remove pub
    pub last_picker: Option<Box<dyn Component>>,
}

impl Compositor {
    pub fn new(area: Rect) -> Self {
        Self {
            layers: Vec::new(),
            area,
            last_picker: None,
        }
    }

    pub fn size(&self) -> Rect {
        self.area
    }

    pub fn resize(&mut self, area: Rect) {
        self.area = area;
    }

    pub fn push(&mut self, mut layer: Box<dyn Component>) {
        let size = self.size();
        // trigger required_size on init
        layer.required_size((size.width, size.height));
        self.layers.push(layer);
    }

    /// Replace a component that has the given `id` with the new layer and if
    /// no component is found, push the layer normally.
    pub fn replace_or_push<T: Component>(&mut self, id: &'static str, layer: T) {
        if let Some(component) = self.find_id(id) {
            *component = layer;
        } else {
            self.push(Box::new(layer))
        }
    }

    pub fn pop(&mut self) -> Option<Box<dyn Component>> {
        self.layers.pop()
    }

    pub fn handle_event(&mut self, event: Event, cx: &mut Context) -> bool {
        // If it is a key event and a macro is being recorded, push the key event to the recording.
        if let (Event::Key(key), Some((_, keys))) = (event, &mut cx.editor.macro_recording) {
            keys.push(key);
        }

        let mut callbacks = Vec::new();
        let mut consumed = false;

        // propagate events through the layers until we either find a layer that consumes it or we
        // run out of layers (event bubbling)
        for layer in self.layers.iter_mut().rev() {
            match layer.handle_event(event, cx) {
                EventResult::Consumed(Some(callback)) => {
                    callbacks.push(callback);
                    consumed = true;
                    break;
                }
                EventResult::Consumed(None) => {
                    consumed = true;
                    break;
                }
                EventResult::Ignored(Some(callback)) => {
                    callbacks.push(callback);
                }
                EventResult::Ignored(None) => {}
            };
        }

        for callback in callbacks {
            callback(self, cx)
        }

        consumed
    }

    pub fn render(&mut self, area: Rect, cx: &mut RenderContext) {
        for layer in &mut self.layers {
            layer.render(area, cx);
        }
    }

    #[cfg(feature = "term")]
    pub fn cursor(&self, area: Rect, editor: &Editor) -> (Option<Position>, CursorKind) {
        for layer in self.layers.iter().rev() {
            if let (Some(pos), kind) = layer.cursor(area, editor) {
                return (Some(pos), kind);
            }
        }
        (None, CursorKind::Hidden)
    }

    pub fn has_component(&self, type_name: &str) -> bool {
        self.layers
            .iter()
            .any(|component| component.type_name() == type_name)
    }

    pub fn find<T: 'static>(&mut self) -> Option<&mut T> {
        let type_name = std::any::type_name::<T>();
        self.layers
            .iter_mut()
            .find(|component| component.type_name() == type_name)
            .and_then(|component| component.as_any_mut().downcast_mut())
    }

    pub fn find_id<T: 'static>(&mut self, id: &'static str) -> Option<&mut T> {
        self.layers
            .iter_mut()
            .find(|component| component.id() == Some(id))
            .and_then(|component| component.as_any_mut().downcast_mut())
    }
}

// View casting, taken straight from Cursive

use std::any::Any;

/// A view that can be downcasted to its concrete type.
///
/// This trait is automatically implemented for any `T: Component`.
pub trait AnyComponent {
    /// Downcast self to a `Any`.
    fn as_any(&self) -> &dyn Any;

    /// Downcast self to a mutable `Any`.
    fn as_any_mut(&mut self) -> &mut dyn Any;

    /// Returns a boxed any from a boxed self.
    ///
    /// Can be used before `Box::downcast()`.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use helix_term::{ui::Text, compositor::Component};
    /// let boxed: Box<dyn Component> = Box::new(Text::new("text".to_string()));
    /// let text: Box<Text> = boxed.as_boxed_any().downcast().unwrap();
    /// ```
    fn as_boxed_any(self: Box<Self>) -> Box<dyn Any>;
}

impl<T: Component> AnyComponent for T {
    /// Downcast self to a `Any`.
    fn as_any(&self) -> &dyn Any {
        self
    }

    /// Downcast self to a mutable `Any`.
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn as_boxed_any(self: Box<Self>) -> Box<dyn Any> {
        self
    }
}

impl dyn AnyComponent {
    /// Attempts to downcast `self` to a concrete type.
    pub fn downcast_ref<T: Any>(&self) -> Option<&T> {
        self.as_any().downcast_ref()
    }

    /// Attempts to downcast `self` to a concrete type.
    pub fn downcast_mut<T: Any>(&mut self) -> Option<&mut T> {
        self.as_any_mut().downcast_mut()
    }

    /// Attempts to downcast `Box<Self>` to a concrete type.
    pub fn downcast<T: Any>(self: Box<Self>) -> Result<Box<T>, Box<Self>> {
        // Do the check here + unwrap, so the error
        // value is `Self` and not `dyn Any`.
        if self.as_any().is::<T>() {
            Ok(self.as_boxed_any().downcast().unwrap())
        } else {
            Err(self)
        }
    }

    /// Checks if this view is of type `T`.
    pub fn is<T: Any>(&mut self) -> bool {
        self.as_any().is::<T>()
    }
}

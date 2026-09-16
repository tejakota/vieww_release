//! Mount, and keep in step.
//!
//! The tree is built once and patched afterwards. A signal write marks its
//! element pending exactly as it does on any other backend; the next animation
//! frame runs the element tree's own rebuild, re-walks, and touches only the
//! parts of the document that actually differ.

use std::cell::RefCell;
use std::rc::Rc;

use vieww_element::ElementTree;
use vieww_widget::WidgetNode;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{Document, Element as DomElement};

use crate::tree::{walk, VNode};

/// What can go wrong before the first frame.
#[derive(Debug)]
pub enum DomError {
    /// No `window`, or no `document` on it.
    NoBrowser(&'static str),
    /// No element with the id the caller named.
    NoHost(String),
    /// The DOM refused something — creating a node, setting an attribute.
    Dom(String),
}

impl std::fmt::Display for DomError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoBrowser(what) => write!(f, "no {what}: this needs a browser"),
            Self::NoHost(id) => write!(f, "no element with id {id:?} to mount into"),
            Self::Dom(message) => write!(f, "the DOM refused: {message}"),
        }
    }
}

impl std::error::Error for DomError {}

impl From<JsValue> for DomError {
    fn from(value: JsValue) -> Self {
        Self::Dom(format!("{value:?}"))
    }
}

/// The `requestAnimationFrame` closure, held by the closure itself so it can
/// re-arm, and by the handle so dropping the handle stops the loop.
type FrameSlot = Rc<RefCell<Option<Closure<dyn FnMut()>>>>;

/// A `Canvas` island's mount callback, queued during a patch and run once the
/// element it names is actually in the document.
type Mount = (String, Rc<dyn Fn(&str)>);

/// A mounted page. Dropping it stops the rebuild loop.
pub struct DomHandle {
    _inner: Rc<RefCell<Inner>>,
    _frame: FrameSlot,
}

struct Inner {
    tree: ElementTree,
    document: Document,
    host: DomElement,
    shadow: Vec<VNode>,
    /// Kept alive for as long as the nodes that listen through them.
    listeners: Vec<Closure<dyn FnMut(web_sys::Event)>>,
}

/// The entry point.
pub struct DomApp;

impl DomApp {
    /// Build `root` into the element with id `host_id`, and keep it there.
    ///
    /// # Errors
    ///
    /// [`DomError`] if there is no browser, no such element, or the document
    /// refuses a node.
    pub fn mount(host_id: &str, root: impl Into<WidgetNode>) -> Result<DomHandle, DomError> {
        let window = web_sys::window().ok_or(DomError::NoBrowser("window"))?;
        let document = window.document().ok_or(DomError::NoBrowser("document"))?;
        let host = document
            .get_element_by_id(host_id)
            .ok_or_else(|| DomError::NoHost(host_id.to_owned()))?;

        let mut tree = ElementTree::new();
        tree.set_root(root);

        let inner = Rc::new(RefCell::new(Inner {
            tree,
            document,
            host,
            shadow: Vec::new(),
            listeners: Vec::new(),
        }));

        {
            let mut guard = inner.borrow_mut();
            let next = guard.render();
            guard.replace_all(&next)?;
            guard.shadow = next;
        }

        // The rebuild loop. `requestAnimationFrame` rather than a callback on
        // the runtime, for the reason the canvas backend gives: the browser
        // already knows when it is worth doing work, and a tab nobody is
        // looking at is not called back at all.
        let frame: FrameSlot = Rc::new(RefCell::new(None));
        let scheduled = frame.clone();
        let weak = Rc::downgrade(&inner);
        let window_for_loop = window.clone();
        *frame.borrow_mut() = Some(Closure::wrap(Box::new(move || {
            if let Some(inner) = weak.upgrade() {
                let mut guard = inner.borrow_mut();
                if guard.tree.rebuild_pending() > 0 {
                    let next = guard.render();
                    let previous = std::mem::take(&mut guard.shadow);
                    let _ = guard.patch_root(&previous, &next);
                    guard.shadow = next;
                }
            }
            if let Some(closure) = scheduled.borrow().as_ref() {
                let _ = window_for_loop.request_animation_frame(closure.as_ref().unchecked_ref());
            }
        }) as Box<dyn FnMut()>));
        if let Some(closure) = frame.borrow().as_ref() {
            window.request_animation_frame(closure.as_ref().unchecked_ref())?;
        }

        Ok(DomHandle {
            _inner: inner,
            _frame: frame,
        })
    }
}

impl Inner {
    fn render(&self) -> Vec<VNode> {
        self.tree
            .root()
            .map_or_else(Vec::new, |root| walk(&self.tree, root))
    }

    fn replace_all(&mut self, nodes: &[VNode]) -> Result<(), DomError> {
        self.host.set_inner_html("");
        self.listeners.clear();
        let mut mounts = Vec::new();
        for node in nodes {
            let created = self.create(node, &mut mounts)?;
            self.host.append_child(&created)?;
        }
        run_mounts(mounts);
        Ok(())
    }

    fn patch_root(&mut self, previous: &[VNode], next: &[VNode]) -> Result<(), DomError> {
        if previous.len() != next.len() {
            return self.replace_all(next);
        }
        let host = self.host.clone();
        let mut mounts = Vec::new();
        for (index, (old, new)) in previous.iter().zip(next).enumerate() {
            let Some(node) = host.child_nodes().item(index as u32) else {
                return self.replace_all(next);
            };
            self.patch(&node, old, new, &mut mounts)?;
        }
        run_mounts(mounts);
        Ok(())
    }

    #[allow(clippy::only_used_in_recursion)]
    fn patch(
        &mut self,
        node: &web_sys::Node,
        old: &VNode,
        new: &VNode,
        mounts: &mut Vec<Mount>,
    ) -> Result<(), DomError> {
        // A canvas is never patched in place: replacing or re-attributing it
        // would restart the application painting into it.
        if old.tag != new.tag || old.canvas.is_some() || new.canvas.is_some() {
            if old.tag != new.tag {
                let replacement = self.create(new, mounts)?;
                if let Some(parent) = node.parent_node() {
                    parent.replace_child(&replacement, node)?;
                }
            }
            return Ok(());
        }
        let Some(element) = node.dyn_ref::<DomElement>() else {
            return Ok(());
        };
        if old.css != new.css {
            element.set_attribute("style", &new.css)?;
        }
        if old.class != new.class {
            match &new.class {
                Some(class) => element.set_attribute("class", class)?,
                None => element.remove_attribute("class")?,
            }
        }
        if old.text != new.text {
            if let Some(text) = &new.text {
                if new.children.is_empty() {
                    element.set_text_content(Some(text));
                }
            }
        }
        if old.children.len() != new.children.len() {
            let created = self.create(new, mounts)?;
            if let Some(parent) = node.parent_node() {
                parent.replace_child(&created, node)?;
            }
            return Ok(());
        }
        for (index, (a, b)) in old.children.iter().zip(&new.children).enumerate() {
            if let Some(child) = node.child_nodes().item(index as u32) {
                self.patch(&child, a, b, mounts)?;
            }
        }
        Ok(())
    }

    fn create(&mut self, node: &VNode, mounts: &mut Vec<Mount>) -> Result<DomElement, DomError> {
        let element = self.document.create_element(node.tag)?;
        if !node.css.is_empty() {
            element.set_attribute("style", &node.css)?;
        }
        if let Some(class) = &node.class {
            element.set_attribute("class", class)?;
        }
        if let Some(id) = &node.id {
            element.set_attribute("id", id)?;
        }
        if let Some(label) = &node.label {
            element.set_attribute(
                if node.tag == "img" {
                    "alt"
                } else {
                    "aria-label"
                },
                label,
            )?;
        }
        if let Some(href) = &node.href {
            element.set_attribute(if node.tag == "img" { "src" } else { "href" }, href)?;
        }
        if let Some((id, mount)) = &node.canvas {
            mounts.push((id.clone(), mount.clone()));
        }
        if let Some(handler) = &node.on_click {
            let handler = handler.clone();
            let closure = Closure::wrap(Box::new(move |_event: web_sys::Event| {
                handler();
            }) as Box<dyn FnMut(web_sys::Event)>);
            element.add_event_listener_with_callback("click", closure.as_ref().unchecked_ref())?;
            self.listeners.push(closure);
        }
        if let Some(text) = &node.text {
            if node.children.is_empty() {
                element.set_text_content(Some(text));
            }
        }
        for child in &node.children {
            let created = self.create(child, mounts)?;
            element.append_child(&created)?;
        }
        Ok(element)
    }
}

/// Canvas islands start after the document has their element, not during the
/// walk that creates it — a `getElementById` from inside `create` would miss.
fn run_mounts(mounts: Vec<Mount>) {
    for (id, mount) in mounts {
        mount(&id);
    }
}

//! Internal DOM types and html5ever `TreeSink` implementation.
//!
//! This module builds a simple in-memory DOM tree from an HTML fragment.
//! It is pure html5ever plumbing — no document-model knowledge.

use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use html5ever::tendril::{StrTendril, TendrilSink};
use html5ever::tree_builder::{ElementFlags, NodeOrText, QuirksMode, TreeSink};
use html5ever::{parse_fragment, Attribute, QualName};
use markup5ever::{local_name, ns};

// ── DOM types ───────────────────────────────────────────

pub(crate) type Handle = Rc<RefCell<Node>>;

pub(crate) enum NodeData {
    Document,
    Text(String),
    Element {
        name: QualName,
        attrs: Vec<Attribute>,
        template_contents: Option<Handle>,
        mathml_annotation_xml_integration_point: bool,
    },
    Comment,
    Doctype,
    ProcessingInstruction,
}

pub(crate) struct Node {
    pub data: NodeData,
    pub parent: Option<Handle>,
    pub children: Vec<Handle>,
}

impl Node {
    pub fn new(data: NodeData) -> Handle {
        Rc::new(RefCell::new(Node {
            data,
            parent: None,
            children: Vec::new(),
        }))
    }
}

// ── TreeSink implementation ─────────────────────────────

struct Sink {
    document: Handle,
    quirks_mode: RefCell<QuirksMode>,
}

impl Sink {
    fn new() -> Self {
        Self {
            document: Node::new(NodeData::Document),
            quirks_mode: RefCell::new(QuirksMode::NoQuirks),
        }
    }

    fn append_child(parent: &Handle, child: &Handle) {
        // Remove from old parent if any.
        {
            let mut child_borrow = child.borrow_mut();
            if let Some(old_parent) = child_borrow.parent.take() {
                old_parent
                    .borrow_mut()
                    .children
                    .retain(|c| !Rc::ptr_eq(c, child));
            }
            child_borrow.parent = Some(Rc::clone(parent));
        }
        parent.borrow_mut().children.push(Rc::clone(child));
    }

    /// If the last child of `parent` is a text node, append to it. Otherwise
    /// create a new text node.
    fn append_text(parent: &Handle, text: &str) {
        let last_is_text = {
            let p = parent.borrow();
            p.children.last().is_some_and(|c| {
                matches!(c.borrow().data, NodeData::Text(_))
            })
        };
        if last_is_text {
            let p = parent.borrow();
            if let Some(last) = p.children.last()
                && let NodeData::Text(ref mut s) = last.borrow_mut().data {
                    s.push_str(text);
                }
        } else {
            let node = Node::new(NodeData::Text(text.to_owned()));
            Self::append_child(parent, &node);
        }
    }
}

impl TreeSink for Sink {
    type Handle = Handle;
    type Output = Handle;
    type ElemName<'a> = &'a QualName;

    fn finish(self) -> Handle {
        self.document
    }

    fn parse_error(&self, _msg: Cow<'static, str>) {}

    fn get_document(&self) -> Handle {
        Rc::clone(&self.document)
    }

    fn elem_name<'a>(&'a self, target: &'a Handle) -> Self::ElemName<'a> {
        // Safety: we only call this on element handles, which live as long as
        // the sink. However the borrow checker doesn't know that, so we use
        // a small unsafe to extend the lifetime — the QualName is heap-allocated
        // inside the Rc<RefCell<Node>> which outlives the borrow.
        //
        // This is the standard pattern for html5ever TreeSink impls that use
        // Rc<RefCell<Node>>.
        let borrow = target.borrow();
        match borrow.data {
            NodeData::Element { ref name, .. } => {
                // SAFETY: The QualName is inside an Rc<RefCell<Node>>. The Rc
                // keeps it alive for the duration of parsing. The returned
                // reference's lifetime is bound to `'a` which is the sink's
                // lifetime, and the sink owns the Rc transitively (through the
                // document tree). html5ever only calls elem_name during parsing
                // while the sink is alive.
                unsafe { &*(name as *const QualName) }
            }
            _ => panic!("elem_name called on non-element"),
        }
    }

    fn create_element(
        &self,
        name: QualName,
        attrs: Vec<Attribute>,
        flags: ElementFlags,
    ) -> Handle {
        Node::new(NodeData::Element {
            template_contents: if flags.template {
                Some(Node::new(NodeData::Document))
            } else {
                None
            },
            mathml_annotation_xml_integration_point: flags
                .mathml_annotation_xml_integration_point,
            name,
            attrs,
        })
    }

    fn create_comment(&self, _text: StrTendril) -> Handle {
        Node::new(NodeData::Comment)
    }

    fn create_pi(&self, _target: StrTendril, _data: StrTendril) -> Handle {
        Node::new(NodeData::ProcessingInstruction)
    }

    fn append(&self, parent: &Handle, child: NodeOrText<Handle>) {
        match child {
            NodeOrText::AppendNode(node) => Self::append_child(parent, &node),
            NodeOrText::AppendText(text) => Self::append_text(parent, &text),
        }
    }

    fn append_based_on_parent_node(
        &self,
        element: &Handle,
        prev_element: &Handle,
        child: NodeOrText<Handle>,
    ) {
        let has_parent = element.borrow().parent.is_some();
        if has_parent {
            self.append_before_sibling(element, child);
        } else {
            self.append(prev_element, child);
        }
    }

    fn append_doctype_to_document(
        &self,
        _name: StrTendril,
        _public_id: StrTendril,
        _system_id: StrTendril,
    ) {
        let node = Node::new(NodeData::Doctype);
        Self::append_child(&self.document, &node);
    }

    fn get_template_contents(&self, target: &Handle) -> Handle {
        let borrow = target.borrow();
        if let NodeData::Element {
            template_contents: Some(ref contents),
            ..
        } = borrow.data
        {
            Rc::clone(contents)
        } else {
            panic!("not a template element")
        }
    }

    fn same_node(&self, x: &Handle, y: &Handle) -> bool {
        Rc::ptr_eq(x, y)
    }

    fn set_quirks_mode(&self, mode: QuirksMode) {
        *self.quirks_mode.borrow_mut() = mode;
    }

    fn append_before_sibling(&self, sibling: &Handle, new_node: NodeOrText<Handle>) {
        let parent = {
            let s = sibling.borrow();
            s.parent.as_ref().map(Rc::clone)
        };
        let Some(parent) = parent else { return };

        let idx = {
            let p = parent.borrow();
            p.children
                .iter()
                .position(|c| Rc::ptr_eq(c, sibling))
        };
        let Some(idx) = idx else { return };

        match new_node {
            NodeOrText::AppendNode(node) => {
                // Remove from old parent
                {
                    let mut nb = node.borrow_mut();
                    if let Some(old_parent) = nb.parent.take() {
                        old_parent
                            .borrow_mut()
                            .children
                            .retain(|c| !Rc::ptr_eq(c, &node));
                    }
                    nb.parent = Some(Rc::clone(&parent));
                }
                parent.borrow_mut().children.insert(idx, node);
            }
            NodeOrText::AppendText(text) => {
                // Check if previous sibling is text
                if idx > 0 {
                    let p = parent.borrow();
                    let prev = &p.children[idx - 1];
                    let is_text = matches!(prev.borrow().data, NodeData::Text(_));
                    if is_text {
                        if let NodeData::Text(ref mut s) = prev.borrow_mut().data {
                            s.push_str(&text);
                        }
                        return;
                    }
                }
                let node = Node::new(NodeData::Text(text.to_string()));
                {
                    node.borrow_mut().parent = Some(Rc::clone(&parent));
                }
                parent.borrow_mut().children.insert(idx, node);
            }
        }
    }

    fn add_attrs_if_missing(&self, target: &Handle, attrs: Vec<Attribute>) {
        let mut borrow = target.borrow_mut();
        if let NodeData::Element {
            attrs: ref mut existing,
            ..
        } = borrow.data
        {
            let names: HashSet<_> = existing.iter().map(|a| a.name.clone()).collect();
            for attr in attrs {
                if !names.contains(&attr.name) {
                    existing.push(attr);
                }
            }
        }
    }

    fn remove_from_parent(&self, target: &Handle) {
        let parent = {
            let mut t = target.borrow_mut();
            t.parent.take()
        };
        if let Some(parent) = parent {
            parent
                .borrow_mut()
                .children
                .retain(|c| !Rc::ptr_eq(c, target));
        }
    }

    fn reparent_children(&self, node: &Handle, new_parent: &Handle) {
        let children: Vec<Handle> = node.borrow_mut().children.drain(..).collect();
        for child in children {
            child.borrow_mut().parent = Some(Rc::clone(new_parent));
            new_parent.borrow_mut().children.push(child);
        }
    }

    fn is_mathml_annotation_xml_integration_point(&self, handle: &Handle) -> bool {
        let borrow = handle.borrow();
        if let NodeData::Element {
            mathml_annotation_xml_integration_point,
            ..
        } = borrow.data
        {
            mathml_annotation_xml_integration_point
        } else {
            false
        }
    }
}

// ── Public entry point ──────────────────────────────────

/// Parse a DOM tree from an HTML fragment.
pub(crate) fn parse_html_fragment(html: &str) -> Handle {
    let sink = Sink::new();
    let context_name = QualName::new(None, ns!(html), local_name!("body"));
    let parser = parse_fragment(sink, Default::default(), context_name, vec![], false);
    parser.one(StrTendril::from(html))
}

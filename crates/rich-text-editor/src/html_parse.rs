//! HTML -> Document parsing via html5ever.
//!
//! Narrow scope: only handles the editor's own HTML subset (drafts, signatures,
//! reply-quoted content). Does NOT handle arbitrary wild HTML -- that's litehtml-rs's job.
//!
//! The parser builds a simple DOM tree via html5ever's `TreeSink` trait, then
//! recursively walks it to produce `Block`s and `StyledRun`s.

use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::Arc;

use html5ever::tendril::{StrTendril, TendrilSink};
use html5ever::tree_builder::{ElementFlags, NodeOrText, QuirksMode, TreeSink};
use html5ever::{parse_fragment, Attribute, QualName};
use markup5ever::{local_name, ns};

use crate::document::{
    Block, Document, HeadingLevel, InlineStyle, StyledRun,
};

// ── DOM types (internal, for html5ever TreeSink) ────────

type Handle = Rc<RefCell<Node>>;

enum NodeData {
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

struct Node {
    data: NodeData,
    parent: Option<Handle>,
    children: Vec<Handle>,
}

impl Node {
    fn new(data: NodeData) -> Handle {
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

// ── Parsing ─────────────────────────────────────────────

/// Parse a DOM tree from an HTML fragment.
fn parse_html_fragment(html: &str) -> Handle {
    let sink = Sink::new();
    let context_name = QualName::new(None, ns!(html), local_name!("body"));
    let parser = parse_fragment(sink, Default::default(), context_name, vec![], false);
    parser.one(StrTendril::from(html))
}

// ── Tag classification ──────────────────────────────────

fn is_block_element(tag: &str) -> bool {
    matches!(
        tag,
        "p" | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "ul"
            | "ol"
            | "li"
            | "blockquote"
            | "div"
            | "hr"
            | "pre"
            | "table"
            | "thead"
            | "tbody"
            | "tr"
            | "td"
            | "th"
            | "img"
    )
}

fn tag_to_inline_style(tag: &str) -> Option<InlineStyle> {
    match tag {
        "strong" | "b" => Some(InlineStyle::BOLD),
        "em" | "i" => Some(InlineStyle::ITALIC),
        "u" => Some(InlineStyle::UNDERLINE),
        "s" | "strike" | "del" => Some(InlineStyle::STRIKETHROUGH),
        _ => None,
    }
}

/// Extract the `href` attribute from an `<a>` element's attributes.
fn get_href(attrs: &[Attribute]) -> Option<String> {
    get_attr(attrs, "href")
}

/// Extract a named attribute value from an element's attributes.
fn get_attr(attrs: &[Attribute], name: &str) -> Option<String> {
    attrs.iter().find_map(|a| {
        if a.name.local.as_ref() == name {
            Some(a.value.to_string())
        } else {
            None
        }
    })
}

// ── Style stack ─────────────────────────────────────────

/// Accumulated inline style while walking inline elements.
#[derive(Clone)]
struct StyleContext {
    style: InlineStyle,
    link: Option<String>,
}

impl StyleContext {
    fn new() -> Self {
        Self {
            style: InlineStyle::empty(),
            link: None,
        }
    }

    fn with_style(&self, extra: InlineStyle) -> Self {
        Self {
            style: self.style | extra,
            link: self.link.clone(),
        }
    }

    fn with_link(&self, href: String) -> Self {
        Self {
            style: self.style,
            link: Some(href),
        }
    }
}

// ── Whitespace collapsing ───────────────────────────────

/// Collapse runs of whitespace into a single space, then trim leading/trailing.
fn collapse_whitespace(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut prev_was_ws = false;
    for ch in text.chars() {
        if ch.is_ascii_whitespace() || ch == '\u{a0}' {
            if !prev_was_ws {
                result.push(' ');
                prev_was_ws = true;
            }
        } else {
            result.push(ch);
            prev_was_ws = false;
        }
    }
    result
}

// ── DOM -> Document conversion ──────────────────────────

/// Collect inline runs from a node and its children, accumulating styles.
fn collect_inline_runs(node: &Handle, ctx: &StyleContext, runs: &mut Vec<StyledRun>) {
    let borrow = node.borrow();
    match borrow.data {
        NodeData::Text(ref text) => {
            let collapsed = collapse_whitespace(text);
            if !collapsed.is_empty() {
                // Try to merge with the previous run if same formatting.
                let can_merge = runs.last().is_some_and(|last| {
                    last.style == ctx.style && last.link == ctx.link
                });
                if can_merge
                    && let Some(last) = runs.last_mut() {
                        last.text.push_str(&collapsed);
                        return;
                    }
                runs.push(StyledRun {
                    text: collapsed,
                    style: ctx.style,
                    link: ctx.link.clone(),
                });
            }
        }
        NodeData::Element {
            ref name,
            ref attrs,
            ..
        } => {
            let tag = name.local.as_ref();

            // If this is a <br>, add a newline-ish break. In inline context
            // we just emit a newline character that will be visible in the run.
            if tag == "br" {
                runs.push(StyledRun {
                    text: "\n".to_owned(),
                    style: ctx.style,
                    link: ctx.link.clone(),
                });
                return;
            }

            // Determine the new style context for children.
            let child_ctx = if tag == "a" {
                if let Some(href) = get_href(attrs) {
                    ctx.with_link(href)
                } else {
                    ctx.clone()
                }
            } else if let Some(inline_style) = tag_to_inline_style(tag) {
                ctx.with_style(inline_style)
            } else {
                // Unknown inline element or block-in-inline (shouldn't happen
                // in our own output, but handle gracefully).
                ctx.clone()
            };

            for child in &borrow.children {
                collect_inline_runs(child, &child_ctx, runs);
            }
        }
        // Skip comments, doctypes, etc.
        _ => {}
    }
}

/// Convert a node to a list of blocks. This is the main recursive descent.
fn node_to_blocks(node: &Handle, blocks: &mut Vec<Block>) {
    let borrow = node.borrow();
    match borrow.data {
        NodeData::Document => {
            for child in &borrow.children {
                node_to_blocks(child, blocks);
            }
        }
        NodeData::Text(ref text) => {
            // Bare text at block level: create a paragraph if non-whitespace.
            let collapsed = collapse_whitespace(text);
            let trimmed = collapsed.trim();
            if !trimmed.is_empty() {
                blocks.push(Block::Paragraph {
                    runs: vec![StyledRun::plain(trimmed)],
                });
            }
        }
        NodeData::Element {
            ref name, ..
        } => {
            let tag = name.local.as_ref();

            match tag {
                "p" => {
                    if tree_has_img(&borrow.children) {
                        collect_blocks_with_inline_images(
                            &borrow.children,
                            blocks,
                            |runs| Block::Paragraph { runs },
                        );
                    } else {
                        let runs = collect_element_runs(node, &borrow.children);
                        blocks.push(Block::Paragraph { runs });
                    }
                }
                "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                    let level = match tag {
                        "h1" => HeadingLevel::H1,
                        "h2" => HeadingLevel::H2,
                        // H4-H6 map to H3 per spec
                        _ => HeadingLevel::H3,
                    };
                    let has_img = tree_has_img(&borrow.children);
                    if has_img {
                        collect_blocks_with_inline_images(
                            &borrow.children,
                            blocks,
                            |runs| Block::Heading { level, runs },
                        );
                    } else {
                        let runs = collect_element_runs(node, &borrow.children);
                        blocks.push(Block::Heading { level, runs });
                    }
                }
                "ul" | "ol" => {
                    let ordered = tag == "ol";
                    drop(borrow);
                    parse_list_to_items(node, ordered, 0, blocks);
                }
                "blockquote" => {
                    let mut inner_blocks = Vec::new();
                    for child in &borrow.children {
                        node_to_blocks(child, &mut inner_blocks);
                    }
                    if inner_blocks.is_empty() {
                        inner_blocks.push(Block::empty_paragraph());
                    }
                    blocks.push(Block::BlockQuote {
                        blocks: inner_blocks.into_iter().map(Arc::new).collect(),
                    });
                }
                "hr" => {
                    blocks.push(Block::HorizontalRule);
                }
                "img" => {
                    if let NodeData::Element { ref attrs, .. } = borrow.data {
                        let src = get_attr(attrs, "src").unwrap_or_default();
                        let alt = get_attr(attrs, "alt").unwrap_or_default();
                        let width = get_attr(attrs, "width").and_then(|w| w.parse().ok());
                        let height = get_attr(attrs, "height").and_then(|h| h.parse().ok());
                        blocks.push(Block::Image {
                            src,
                            alt,
                            width,
                            height,
                        });
                    }
                }
                "br" => {
                    // A <br> at block level creates an empty paragraph.
                    blocks.push(Block::empty_paragraph());
                }
                "div" => {
                    // <div> is a block container -- recurse into children.
                    // If children contain only inline content, wrap in paragraph.
                    let has_block_children = borrow.children.iter().any(|c| {
                        let cb = c.borrow();
                        if let NodeData::Element { ref name, .. } = cb.data {
                            is_block_element(name.local.as_ref())
                        } else {
                            false
                        }
                    });

                    if has_block_children {
                        for child in &borrow.children {
                            node_to_blocks(child, blocks);
                        }
                    } else {
                        // All inline children -- wrap in a paragraph.
                        let runs = collect_element_runs(node, &borrow.children);
                        blocks.push(Block::Paragraph { runs });
                    }
                }
                // Tables flatten to text paragraphs.
                "table" | "thead" | "tbody" | "tr" => {
                    for child in &borrow.children {
                        node_to_blocks(child, blocks);
                    }
                }
                "td" | "th" => {
                    if tree_has_img(&borrow.children) {
                        collect_blocks_with_inline_images(
                            &borrow.children,
                            blocks,
                            |runs| Block::Paragraph { runs },
                        );
                    } else {
                        let runs = collect_element_runs(node, &borrow.children);
                        if !runs_are_empty(&runs) {
                            blocks.push(Block::Paragraph { runs });
                        }
                    }
                }
                "pre" => {
                    // Preserve text as-is (no whitespace collapsing).
                    let mut text = String::new();
                    collect_pre_text(node, &mut text);
                    blocks.push(Block::Paragraph {
                        runs: vec![StyledRun::plain(text)],
                    });
                }
                // li outside a list context (shouldn't happen from our output)
                "li" => {
                    for child in &borrow.children {
                        node_to_blocks(child, blocks);
                    }
                }
                // Transparent wrappers from the html5ever fragment parser.
                "html" | "head" | "body" => {
                    for child in &borrow.children {
                        node_to_blocks(child, blocks);
                    }
                }
                _ => {
                    // Unknown element. If block-level, recurse. If inline, wrap
                    // in paragraph.
                    if is_block_element(tag) {
                        for child in &borrow.children {
                            node_to_blocks(child, blocks);
                        }
                    } else {
                        // Inline element at block level: collect as runs in a paragraph.
                        let ctx = StyleContext::new();
                        let mut runs = Vec::new();
                        // Drop borrow before calling collect_inline_runs
                        // which needs to borrow the node.
                        drop(borrow);
                        collect_inline_runs(node, &ctx, &mut runs);
                        trim_runs(&mut runs);
                        if runs.is_empty() {
                            runs.push(StyledRun::plain(String::new()));
                        }
                        if !runs_are_empty(&runs) {
                            blocks.push(Block::Paragraph { runs });
                        }
                    }
                }
            }
        }
        // Comments, doctypes, PIs: skip
        _ => {}
    }
}

/// Parse a `<ul>`/`<ol>` element, flattening `<li>` children into
/// `Block::ListItem` blocks with the given indent level.
fn parse_list_to_items(
    list_node: &Handle,
    ordered: bool,
    indent_level: u8,
    blocks: &mut Vec<Block>,
) {
    let borrow = list_node.borrow();
    let mut found_any = false;
    for child in &borrow.children {
        let child_borrow = child.borrow();
        if let NodeData::Element { ref name, .. } = child_borrow.data
            && name.local.as_ref() == "li"
        {
            drop(child_borrow);
            parse_li_to_items(child, ordered, indent_level, blocks);
            found_any = true;
        }
        // Skip non-li children (whitespace text nodes, etc.)
    }
    if !found_any {
        blocks.push(Block::ListItem {
            ordered,
            indent_level,
            runs: vec![StyledRun::plain(String::new())],
        });
    }
}

/// Parse a single `<li>` element into one or more `Block::ListItem` blocks.
///
/// Inline content becomes a single `ListItem`. Nested `<ul>`/`<ol>` elements
/// recurse with `indent_level + 1`.
fn parse_li_to_items(
    li_node: &Handle,
    ordered: bool,
    indent_level: u8,
    blocks: &mut Vec<Block>,
) {
    let borrow = li_node.borrow();

    // Separate children into inline content and nested lists.
    // Inline content before the first nested list becomes this item's runs.
    // Nested lists recurse.
    let has_nested_list = borrow.children.iter().any(|c| {
        let cb = c.borrow();
        if let NodeData::Element { ref name, .. } = cb.data {
            let tag = name.local.as_ref();
            return tag == "ul" || tag == "ol";
        }
        false
    });

    if !has_nested_list {
        // Simple case: all inline content.
        drop(borrow);
        let runs = collect_element_runs(li_node, &li_node.borrow().children);
        blocks.push(Block::ListItem {
            ordered,
            indent_level,
            runs,
        });
        return;
    }

    // Mixed content: collect inline runs before each nested list,
    // emit a ListItem for the inline parts, then recurse for nested lists.
    let mut inline_children: Vec<Handle> = Vec::new();
    let mut emitted_inline = false;

    for child in &borrow.children {
        let child_borrow = child.borrow();
        let is_nested_list = if let NodeData::Element { ref name, .. } = child_borrow.data {
            let tag = name.local.as_ref();
            tag == "ul" || tag == "ol"
        } else {
            false
        };

        if is_nested_list {
            // Flush pending inline children as a ListItem.
            if !emitted_inline {
                let runs = collect_inline_children(&inline_children);
                blocks.push(Block::ListItem {
                    ordered,
                    indent_level,
                    runs,
                });
                emitted_inline = true;
            }
            inline_children.clear();

            // Determine ordering of the nested list.
            let nested_ordered = if let NodeData::Element { ref name, .. } = child_borrow.data {
                name.local.as_ref() == "ol"
            } else {
                false
            };
            drop(child_borrow);
            parse_list_to_items(child, nested_ordered, indent_level + 1, blocks);
        } else {
            drop(child_borrow);
            inline_children.push(Rc::clone(child));
        }
    }

    // If we never emitted an inline item (no nested list was first child),
    // emit whatever inline content we have.
    if !emitted_inline {
        let runs = collect_inline_children(&inline_children);
        blocks.push(Block::ListItem {
            ordered,
            indent_level,
            runs,
        });
    }
}

/// Collect inline runs from a list of child handles.
fn collect_inline_children(children: &[Handle]) -> Vec<StyledRun> {
    let ctx = StyleContext::new();
    let mut runs = Vec::new();
    for child in children {
        collect_inline_runs(child, &ctx, &mut runs);
    }
    trim_runs(&mut runs);
    if runs.is_empty() {
        runs.push(StyledRun::plain(String::new()));
    }
    runs
}

/// Collect blocks from children that may contain `<img>` elements mixed with
/// inline content. Flushes accumulated inline runs as block-level elements
/// (using `wrap_runs`) whenever an `<img>` is encountered.
/// Check whether a node tree contains any `<img>` elements at any depth.
fn tree_has_img(children: &[Handle]) -> bool {
    children.iter().any(|c| {
        let cb = c.borrow();
        if let NodeData::Element { ref name, .. } = cb.data {
            if name.local.as_ref() == "img" {
                return true;
            }
            // Recurse into inline wrappers.
            if !is_block_element(name.local.as_ref()) {
                return tree_has_img(&cb.children);
            }
        }
        false
    })
}

/// An item produced during mixed inline/image collection.
enum InlineOrImage {
    Run(StyledRun),
    Image {
        src: String,
        alt: String,
        width: Option<u32>,
        height: Option<u32>,
    },
}

/// Collect inline runs and images from a node, handling `<img>` at any
/// nesting depth within inline wrappers.
fn collect_inline_or_images(
    node: &Handle,
    ctx: &StyleContext,
    out: &mut Vec<InlineOrImage>,
) {
    let borrow = node.borrow();
    match borrow.data {
        NodeData::Text(ref text) => {
            let collapsed = collapse_whitespace(text);
            if !collapsed.is_empty() {
                // Try to merge with the previous run if same formatting.
                let can_merge = matches!(out.last(), Some(InlineOrImage::Run(last)) if last.style == ctx.style && last.link == ctx.link);
                if can_merge
                    && let Some(InlineOrImage::Run(last)) = out.last_mut()
                {
                    last.text.push_str(&collapsed);
                    return;
                }
                out.push(InlineOrImage::Run(StyledRun {
                    text: collapsed,
                    style: ctx.style,
                    link: ctx.link.clone(),
                }));
            }
        }
        NodeData::Element {
            ref name,
            ref attrs,
            ..
        } => {
            let tag = name.local.as_ref();

            if tag == "br" {
                out.push(InlineOrImage::Run(StyledRun {
                    text: "\n".to_owned(),
                    style: ctx.style,
                    link: ctx.link.clone(),
                }));
                return;
            }

            if tag == "img" {
                out.push(InlineOrImage::Image {
                    src: get_attr(attrs, "src").unwrap_or_default(),
                    alt: get_attr(attrs, "alt").unwrap_or_default(),
                    width: get_attr(attrs, "width").and_then(|w| w.parse().ok()),
                    height: get_attr(attrs, "height").and_then(|h| h.parse().ok()),
                });
                return;
            }

            let child_ctx = if tag == "a" {
                if let Some(href) = get_href(attrs) {
                    ctx.with_link(href)
                } else {
                    ctx.clone()
                }
            } else if let Some(inline_style) = tag_to_inline_style(tag) {
                ctx.with_style(inline_style)
            } else {
                ctx.clone()
            };

            for child in &borrow.children {
                collect_inline_or_images(child, &child_ctx, out);
            }
        }
        _ => {}
    }
}

/// Collect blocks from children that may contain `<img>` mixed with inline
/// content at any nesting depth. Flushes pending runs as blocks when an
/// image is encountered.
fn collect_blocks_with_inline_images(
    children: &[Handle],
    blocks: &mut Vec<Block>,
    wrap_runs: impl Fn(Vec<StyledRun>) -> Block,
) {
    let ctx = StyleContext::new();
    let mut items: Vec<InlineOrImage> = Vec::new();

    for child in children {
        collect_inline_or_images(child, &ctx, &mut items);
    }

    // Convert the mixed stream into blocks: consecutive runs become one
    // block, images become Image blocks.
    let mut pending_runs: Vec<StyledRun> = Vec::new();
    for item in items {
        match item {
            InlineOrImage::Run(run) => pending_runs.push(run),
            InlineOrImage::Image {
                src,
                alt,
                width,
                height,
            } => {
                flush_pending_runs(&mut pending_runs, blocks, &wrap_runs);
                blocks.push(Block::Image {
                    src,
                    alt,
                    width,
                    height,
                });
            }
        }
    }
    flush_pending_runs(&mut pending_runs, blocks, &wrap_runs);
}

/// Flush accumulated inline runs into a block, if non-empty.
fn flush_pending_runs(
    runs: &mut Vec<StyledRun>,
    blocks: &mut Vec<Block>,
    wrap_runs: &impl Fn(Vec<StyledRun>) -> Block,
) {
    if runs.is_empty() {
        return;
    }
    let mut flushed = std::mem::take(runs);
    trim_runs(&mut flushed);
    if flushed.is_empty() {
        flushed.push(StyledRun::plain(String::new()));
    }
    if !runs_are_empty(&flushed) {
        blocks.push(wrap_runs(flushed));
    }
}

/// Collect inline runs from an element's children.
fn collect_element_runs(
    _parent: &Handle,
    children: &[Handle],
) -> Vec<StyledRun> {
    let ctx = StyleContext::new();
    let mut runs = Vec::new();
    for child in children {
        collect_inline_runs(child, &ctx, &mut runs);
    }
    trim_runs(&mut runs);
    if runs.is_empty() {
        runs.push(StyledRun::plain(String::new()));
    }
    runs
}

/// Collect text from a `<pre>` element without whitespace collapsing.
fn collect_pre_text(node: &Handle, buf: &mut String) {
    let borrow = node.borrow();
    match borrow.data {
        NodeData::Text(ref text) => buf.push_str(text),
        NodeData::Element { .. } => {
            for child in &borrow.children {
                collect_pre_text(child, buf);
            }
        }
        _ => {}
    }
}

/// Trim leading whitespace from the first run and trailing whitespace from the
/// last run.
fn trim_runs(runs: &mut Vec<StyledRun>) {
    if let Some(first) = runs.first_mut() {
        let trimmed = first.text.trim_start().to_owned();
        first.text = trimmed;
    }
    if let Some(last) = runs.last_mut() {
        let trimmed = last.text.trim_end().to_owned();
        last.text = trimmed;
    }
    // Remove now-empty runs from the edges.
    while runs.len() > 1 && runs.first().is_some_and(|r| r.text.is_empty()) {
        runs.remove(0);
    }
    while runs.len() > 1 && runs.last().is_some_and(|r| r.text.is_empty()) {
        runs.pop();
    }
}

/// Check if a run list is effectively empty (all runs are empty or whitespace).
fn runs_are_empty(runs: &[StyledRun]) -> bool {
    runs.iter().all(|r| r.text.trim().is_empty())
}

// ── Public API ──────────────────────────────────────────

/// Parse HTML into a Document.
///
/// This parser only handles the editor's own output: `<p>`, `<h1>`-`<h6>`,
/// `<ul>`, `<ol>`, `<li>`, `<blockquote>`, `<hr>`, and inline formatting tags.
/// Unknown block elements become paragraphs; unknown inline elements pass
/// through content.
///
/// If the input is empty or unparseable, returns `Document::new()` (single
/// empty paragraph).
pub fn from_html(html: &str) -> Document {
    if html.trim().is_empty() {
        return Document::new();
    }

    let dom = parse_html_fragment(html);
    let mut blocks = Vec::new();

    // The fragment parser wraps content in an <html> element. Walk into it.
    let root = dom.borrow();
    for child in &root.children {
        node_to_blocks(child, &mut blocks);
    }

    if blocks.is_empty() {
        return Document::new();
    }

    Document::from_blocks(blocks)
}

// ── Tests ───────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Block, HeadingLevel, InlineStyle, StyledRun};

    #[test]
    fn empty_string_produces_empty_document() {
        let doc = from_html("");
        assert_eq!(doc.block_count(), 1);
        assert_eq!(doc.block(0), Some(&Block::empty_paragraph()));
    }

    #[test]
    fn whitespace_only_produces_empty_document() {
        let doc = from_html("   \n\t  ");
        assert_eq!(doc.block_count(), 1);
        assert_eq!(doc.block(0), Some(&Block::empty_paragraph()));
    }

    #[test]
    fn simple_paragraph() {
        let doc = from_html("<p>hello</p>");
        assert_eq!(doc.block_count(), 1);
        let block = doc.block(0).expect("should have block");
        assert_eq!(
            *block,
            Block::Paragraph {
                runs: vec![StyledRun::plain("hello")]
            }
        );
    }

    #[test]
    fn paragraph_with_bold() {
        let doc = from_html("<p><strong>bold</strong> normal</p>");
        assert_eq!(doc.block_count(), 1);
        let runs = doc.block(0).and_then(Block::runs).expect("should have runs");
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].text, "bold");
        assert_eq!(runs[0].style, InlineStyle::BOLD);
        assert_eq!(runs[1].text, " normal");
        assert_eq!(runs[1].style, InlineStyle::empty());
    }

    #[test]
    fn heading_h1() {
        let doc = from_html("<h1>Title</h1>");
        assert_eq!(doc.block_count(), 1);
        assert_eq!(
            *doc.block(0).expect("block"),
            Block::Heading {
                level: HeadingLevel::H1,
                runs: vec![StyledRun::plain("Title")]
            }
        );
    }

    #[test]
    fn heading_h2() {
        let doc = from_html("<h2>Subtitle</h2>");
        assert_eq!(
            *doc.block(0).expect("block"),
            Block::Heading {
                level: HeadingLevel::H2,
                runs: vec![StyledRun::plain("Subtitle")]
            }
        );
    }

    #[test]
    fn heading_h4_maps_to_h3() {
        let doc = from_html("<h4>Section</h4>");
        assert_eq!(
            *doc.block(0).expect("block"),
            Block::Heading {
                level: HeadingLevel::H3,
                runs: vec![StyledRun::plain("Section")]
            }
        );
    }

    #[test]
    fn heading_h6_maps_to_h3() {
        let doc = from_html("<h6>Deep</h6>");
        assert_eq!(
            *doc.block(0).expect("block"),
            Block::Heading {
                level: HeadingLevel::H3,
                runs: vec![StyledRun::plain("Deep")]
            }
        );
    }

    #[test]
    fn unordered_list() {
        let doc = from_html("<ul><li>one</li><li>two</li></ul>");
        assert_eq!(doc.block_count(), 2);
        if let Block::ListItem { ordered, runs, indent_level } = doc.block(0).expect("block") {
            assert!(!ordered);
            assert_eq!(*indent_level, 0);
            assert_eq!(runs[0].text, "one");
        } else {
            panic!("expected ListItem");
        }
        if let Block::ListItem { ordered, runs, .. } = doc.block(1).expect("block") {
            assert!(!ordered);
            assert_eq!(runs[0].text, "two");
        } else {
            panic!("expected ListItem");
        }
    }

    #[test]
    fn ordered_list() {
        let doc = from_html("<ol><li>first</li></ol>");
        assert_eq!(doc.block_count(), 1);
        if let Block::ListItem { ordered, runs, .. } = doc.block(0).expect("block") {
            assert!(ordered);
            assert_eq!(runs[0].text, "first");
        } else {
            panic!("expected ListItem");
        }
    }

    #[test]
    fn blockquote() {
        let doc = from_html("<blockquote><p>quoted</p></blockquote>");
        assert_eq!(doc.block_count(), 1);
        let block = doc.block(0).expect("block");
        if let Block::BlockQuote { blocks } = block {
            assert_eq!(blocks.len(), 1);
            assert_eq!(blocks[0].flattened_text(), "quoted");
        } else {
            panic!("expected BlockQuote, got {block:?}");
        }
    }

    #[test]
    fn horizontal_rule() {
        let doc = from_html("<hr>");
        assert_eq!(doc.block_count(), 1);
        assert_eq!(*doc.block(0).expect("block"), Block::HorizontalRule);
    }

    #[test]
    fn nested_styles_bold_italic() {
        let doc = from_html("<p><strong><em>bold italic</em></strong></p>");
        let runs = doc.block(0).and_then(Block::runs).expect("runs");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].text, "bold italic");
        assert_eq!(
            runs[0].style,
            InlineStyle::BOLD | InlineStyle::ITALIC
        );
    }

    #[test]
    fn link() {
        let doc = from_html(r#"<p><a href="https://example.com">click</a></p>"#);
        let runs = doc.block(0).and_then(Block::runs).expect("runs");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].text, "click");
        assert_eq!(runs[0].link.as_deref(), Some("https://example.com"));
    }

    #[test]
    fn link_with_bold() {
        let doc =
            from_html(r#"<p><a href="https://example.com"><strong>bold link</strong></a></p>"#);
        let runs = doc.block(0).and_then(Block::runs).expect("runs");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].text, "bold link");
        assert_eq!(runs[0].style, InlineStyle::BOLD);
        assert_eq!(runs[0].link.as_deref(), Some("https://example.com"));
    }

    #[test]
    fn unknown_inline_tag_passes_through() {
        let doc = from_html("<p><span>text</span></p>");
        let runs = doc.block(0).and_then(Block::runs).expect("runs");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].text, "text");
        assert_eq!(runs[0].style, InlineStyle::empty());
    }

    #[test]
    fn bare_text_without_tags() {
        let doc = from_html("hello world");
        assert_eq!(doc.block_count(), 1);
        assert_eq!(
            doc.block(0).expect("block").flattened_text(),
            "hello world"
        );
    }

    #[test]
    fn multiple_paragraphs() {
        let doc = from_html("<p>one</p><p>two</p><p>three</p>");
        assert_eq!(doc.block_count(), 3);
        assert_eq!(doc.block(0).expect("b").flattened_text(), "one");
        assert_eq!(doc.block(1).expect("b").flattened_text(), "two");
        assert_eq!(doc.block(2).expect("b").flattened_text(), "three");
    }

    #[test]
    fn mixed_inline_styles() {
        let doc = from_html("<p>normal <b>bold</b> <i>italic</i> <u>underline</u> <s>strike</s></p>");
        let runs = doc.block(0).and_then(Block::runs).expect("runs");
        assert_eq!(runs[0].text, "normal ");
        assert_eq!(runs[0].style, InlineStyle::empty());
        assert_eq!(runs[1].text, "bold");
        assert_eq!(runs[1].style, InlineStyle::BOLD);
        // Space between bold and italic
        assert_eq!(runs[2].text, " ");
        assert_eq!(runs[3].text, "italic");
        assert_eq!(runs[3].style, InlineStyle::ITALIC);
        assert_eq!(runs[4].text, " ");
        assert_eq!(runs[5].text, "underline");
        assert_eq!(runs[5].style, InlineStyle::UNDERLINE);
        assert_eq!(runs[6].text, " ");
        assert_eq!(runs[7].text, "strike");
        assert_eq!(runs[7].style, InlineStyle::STRIKETHROUGH);
    }

    #[test]
    fn del_and_strike_tags() {
        let doc = from_html("<p><del>deleted</del> <strike>struck</strike></p>");
        let runs = doc.block(0).and_then(Block::runs).expect("runs");
        assert_eq!(runs[0].text, "deleted");
        assert_eq!(runs[0].style, InlineStyle::STRIKETHROUGH);
        assert_eq!(runs[2].text, "struck");
        assert_eq!(runs[2].style, InlineStyle::STRIKETHROUGH);
    }

    #[test]
    fn whitespace_collapsing() {
        let doc = from_html("<p>  hello   world  </p>");
        let text = doc.block(0).expect("block").flattened_text();
        assert_eq!(text, "hello world");
    }

    #[test]
    fn nested_blockquote() {
        let doc = from_html(
            "<blockquote><p>outer</p><blockquote><p>inner</p></blockquote></blockquote>",
        );
        let block = doc.block(0).expect("block");
        if let Block::BlockQuote { blocks } = block {
            assert_eq!(blocks.len(), 2);
            assert_eq!(blocks[0].flattened_text(), "outer");
            if let Block::BlockQuote {
                blocks: inner_blocks,
            } = blocks[1].as_ref()
            {
                assert_eq!(inner_blocks[0].flattened_text(), "inner");
            } else {
                panic!("expected nested BlockQuote");
            }
        } else {
            panic!("expected BlockQuote");
        }
    }

    #[test]
    fn hr_between_paragraphs() {
        let doc = from_html("<p>above</p><hr><p>below</p>");
        assert_eq!(doc.block_count(), 3);
        assert_eq!(doc.block(0).expect("b").flattened_text(), "above");
        assert_eq!(*doc.block(1).expect("b"), Block::HorizontalRule);
        assert_eq!(doc.block(2).expect("b").flattened_text(), "below");
    }

    #[test]
    fn div_with_inline_content() {
        let doc = from_html("<div>hello</div>");
        assert_eq!(doc.block_count(), 1);
        assert_eq!(doc.block(0).expect("b").flattened_text(), "hello");
    }

    #[test]
    fn div_with_block_children() {
        let doc = from_html("<div><p>first</p><p>second</p></div>");
        assert_eq!(doc.block_count(), 2);
        assert_eq!(doc.block(0).expect("b").flattened_text(), "first");
        assert_eq!(doc.block(1).expect("b").flattened_text(), "second");
    }

    #[test]
    fn html_entities_decoded() {
        let doc = from_html("<p>x &lt; y &amp; y &gt; z &quot;quoted&quot;</p>");
        assert_eq!(
            doc.block(0).expect("b").flattened_text(),
            "x < y & y > z \"quoted\""
        );
    }

    #[test]
    fn round_trip_simple() {
        use crate::html_serialize::to_html;
        let html = "<p>Hello, <strong>world</strong>!</p>";
        let doc = from_html(html);
        let output = to_html(&doc);
        assert_eq!(output, html);
    }

    #[test]
    fn round_trip_heading() {
        use crate::html_serialize::to_html;
        let html = "<h1>Title</h1>";
        let doc = from_html(html);
        assert_eq!(to_html(&doc), html);
    }

    #[test]
    fn round_trip_list() {
        use crate::html_serialize::to_html;
        // Parse a list, serialize it, parse again, serialize again — should be stable.
        let html = "<ul><li>one</li><li>two</li></ul>";
        let doc = from_html(html);
        let output = to_html(&doc);
        assert_eq!(output, html);
    }

    #[test]
    fn round_trip_blockquote() {
        use crate::html_serialize::to_html;
        let html = "<blockquote><p>quoted</p></blockquote>";
        let doc = from_html(html);
        assert_eq!(to_html(&doc), html);
    }

    #[test]
    fn round_trip_hr() {
        use crate::html_serialize::to_html;
        let html = "<p>above</p><hr><p>below</p>";
        let doc = from_html(html);
        assert_eq!(to_html(&doc), html);
    }

    #[test]
    fn round_trip_all_styles() {
        use crate::html_serialize::to_html;
        let html = "<p><a href=\"https://example.com\"><strong><em><u><s>styled</s></u></em></strong></a></p>";
        let doc = from_html(html);
        assert_eq!(to_html(&doc), html);
    }

    #[test]
    fn round_trip_mixed_document() {
        use crate::html_serialize::to_html;
        let html = "<h1><strong>Welcome</strong></h1>\
                     <p>Some intro text.</p>\
                     <ul><li>Point A</li><li>Point B</li></ul>\
                     <hr>\
                     <blockquote><p>A wise quote.</p></blockquote>";
        let doc = from_html(html);
        assert_eq!(to_html(&doc), html);
    }

    #[test]
    fn nested_list_round_trip() {
        use crate::html_serialize::to_html;
        let html = "<ol><li>outer item<ul><li>nested-a</li><li>nested-b</li></ul></li><li>second outer</li></ol>";
        let doc = from_html(html);
        assert_eq!(to_html(&doc), html);
    }

    #[test]
    fn unicode_content() {
        let doc = from_html("<p>Héllo wörld \u{1f30d}</p>");
        assert_eq!(
            doc.block(0).expect("b").flattened_text(),
            "Héllo wörld \u{1f30d}"
        );
    }

    #[test]
    fn empty_paragraph_tag() {
        let doc = from_html("<p></p>");
        assert_eq!(doc.block_count(), 1);
        let block = doc.block(0).expect("block");
        assert_eq!(
            *block,
            Block::Paragraph {
                runs: vec![StyledRun::plain("")]
            }
        );
    }

    #[test]
    fn image_block_parsed() {
        let doc = from_html(r#"<img src="https://example.com/img.png" alt="A photo">"#);
        assert_eq!(doc.block_count(), 1);
        let block = doc.block(0).expect("block");
        if let Block::Image { src, alt, width, height } = block {
            assert_eq!(src, "https://example.com/img.png");
            assert_eq!(alt, "A photo");
            assert_eq!(*width, None);
            assert_eq!(*height, None);
        } else {
            panic!("expected Image block, got {block:?}");
        }
    }

    #[test]
    fn image_block_with_dimensions() {
        let doc = from_html(r#"<img src="cid:abc" alt="logo" width="100" height="50">"#);
        assert_eq!(doc.block_count(), 1);
        let block = doc.block(0).expect("block");
        if let Block::Image { src, alt, width, height } = block {
            assert_eq!(src, "cid:abc");
            assert_eq!(alt, "logo");
            assert_eq!(*width, Some(100));
            assert_eq!(*height, Some(50));
        } else {
            panic!("expected Image block, got {block:?}");
        }
    }

    #[test]
    fn image_inside_paragraph_splits_into_blocks() {
        let doc = from_html(r#"<p>before<img src="test.png" alt="img">after</p>"#);
        // Should produce: paragraph("before"), Image, paragraph("after")
        assert!(doc.block_count() >= 2, "got {} blocks", doc.block_count());
        let has_image = (0..doc.block_count())
            .any(|i| matches!(doc.block(i), Some(Block::Image { .. })));
        assert!(has_image, "should contain an Image block");
    }

    #[test]
    fn round_trip_image() {
        use crate::html_serialize::to_html;
        let html = r#"<img src="https://example.com/img.png" alt="A photo" width="100" height="50">"#;
        let doc = from_html(html);
        assert_eq!(to_html(&doc), html);
    }

    #[test]
    fn round_trip_image_minimal() {
        use crate::html_serialize::to_html;
        let html = r#"<img src="cid:abc">"#;
        let doc = from_html(html);
        assert_eq!(to_html(&doc), html);
    }

    #[test]
    fn image_inside_inline_wrapper_in_heading() {
        // <img> wrapped in <a> inside a heading — must not be dropped.
        let doc = from_html(r#"<h1>before<a href="https://x.com"><img src="logo.png" alt="logo"></a>after</h1>"#);
        // Should produce: heading "before", image block, heading "after"
        assert!(doc.block_count() >= 2, "got {} blocks", doc.block_count());
        let has_image = (0..doc.block_count()).any(|i| {
            matches!(doc.block(i), Some(Block::Image { .. }))
        });
        assert!(has_image, "expected an Image block, got: {:?}",
            (0..doc.block_count()).map(|i| doc.block(i).map(Block::kind)).collect::<Vec<_>>());
    }

    #[test]
    fn image_inside_strong_in_paragraph() {
        // <img> wrapped in <strong> inside a paragraph.
        let doc = from_html(r#"<p><strong><img src="pic.jpg" alt="pic"></strong></p>"#);
        let has_image = (0..doc.block_count()).any(|i| {
            matches!(doc.block(i), Some(Block::Image { .. }))
        });
        assert!(has_image, "expected an Image block");
    }

    #[test]
    fn list_item_with_nested_blocks() {
        let doc = from_html("<ul><li><p>text</p><ul><li>nested</li></ul></li></ul>");
        // Flattened model: first item at indent 0, nested item at indent 1.
        assert_eq!(doc.block_count(), 2);
        if let Block::ListItem { indent_level, runs, .. } = doc.block(0).expect("block") {
            assert_eq!(*indent_level, 0);
            assert_eq!(runs[0].text, "text");
        } else {
            panic!("expected ListItem at indent 0");
        }
        if let Block::ListItem { indent_level, runs, .. } = doc.block(1).expect("block") {
            assert_eq!(*indent_level, 1);
            assert_eq!(runs[0].text, "nested");
        } else {
            panic!("expected ListItem at indent 1");
        }
    }

    #[test]
    fn html_parse_nested_list_indent_levels() {
        let doc = from_html("<ul><li>a</li><li>b<ol><li>b1</li><li>b2</li></ol></li><li>c</li></ul>");
        // Should produce: a(0), b(0), b1(1), b2(1), c(0)
        assert_eq!(doc.block_count(), 5);
        let expected = [
            ("a", 0u8, false),
            ("b", 0, false),
            ("b1", 1, true),
            ("b2", 1, true),
            ("c", 0, false),
        ];
        for (i, (text, indent, ordered)) in expected.iter().enumerate() {
            if let Block::ListItem { indent_level, runs, ordered: o } = doc.block(i).expect("block") {
                assert_eq!(runs[0].text, *text, "block {i} text");
                assert_eq!(*indent_level, *indent, "block {i} indent");
                assert_eq!(*o, *ordered, "block {i} ordered");
            } else {
                panic!("block {i}: expected ListItem");
            }
        }
    }
}

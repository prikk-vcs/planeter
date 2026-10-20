//! A small DOM over `html5ever` (a dev-dependency already in the lock through `ammonia`): the HTML5 tree
//! construction algorithm, so the checks see what a browser sees, not what a regex guesses.

use std::borrow::Cow;
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use html5ever::interface::{ElementFlags, NodeOrText, QuirksMode, TreeSink};
use html5ever::tendril::{StrTendril, TendrilSink};
use html5ever::{Attribute, ExpandedName, ParseOpts, QualName, parse_document};

pub type Handle = Rc<Node>;

#[derive(Debug)]
pub enum Data {
    Document,
    Doctype,
    Text(RefCell<String>),
    Comment,
    Element {
        name: QualName,
        attrs: RefCell<Vec<(String, String)>>,
    },
}

pub struct Node {
    pub parent: Cell<Option<Weak<Node>>>,
    pub children: RefCell<Vec<Handle>>,
    pub data: Data,
    pub line: Cell<u64>,
}

fn node(data: Data) -> Handle {
    Rc::new(Node {
        parent: Cell::new(None),
        children: RefCell::new(Vec::new()),
        data,
        line: Cell::new(0),
    })
}

impl Node {
    pub fn tag(&self) -> Option<&str> {
        match &self.data {
            Data::Element { name, .. } => Some(&name.local),
            _ => None,
        }
    }
    pub fn attr(&self, key: &str) -> Option<String> {
        match &self.data {
            Data::Element { attrs, .. } => attrs
                .borrow()
                .iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.clone()),
            _ => None,
        }
    }
    pub fn has_attr(&self, key: &str) -> bool {
        self.attr(key).is_some()
    }
    pub fn attr_names(&self) -> Vec<String> {
        match &self.data {
            Data::Element { attrs, .. } => attrs.borrow().iter().map(|(k, _)| k.clone()).collect(),
            _ => Vec::new(),
        }
    }
    pub fn classes(&self) -> Vec<String> {
        self.attr("class")
            .unwrap_or_default()
            .split_whitespace()
            .map(str::to_owned)
            .collect()
    }
    pub fn parent(&self) -> Option<Handle> {
        let p = self.parent.take();
        self.parent.set(p.clone());
        p.and_then(|w| w.upgrade())
    }
    pub fn ancestors(&self) -> Vec<Handle> {
        let mut out = Vec::new();
        let mut cur = self.parent();
        while let Some(p) = cur {
            cur = p.parent();
            out.push(p);
        }
        out
    }
    /// All text below this node, whitespace collapsed.
    pub fn text(&self) -> String {
        let mut s = String::new();
        collect_text(self, &mut s);
        s.split_whitespace().collect::<Vec<_>>().join(" ")
    }
    pub fn element_children(&self) -> Vec<Handle> {
        self.children
            .borrow()
            .iter()
            .filter(|c| c.tag().is_some())
            .cloned()
            .collect()
    }
    /// Pre-order walk of this node and everything below it.
    pub fn descendants(self: &Handle) -> Vec<Handle> {
        let mut out = Vec::new();
        walk(self, &mut out);
        out
    }
    /// `<tag attrs…>` for messages.
    pub fn describe(&self) -> String {
        match &self.data {
            Data::Element { attrs, .. } => {
                let a: Vec<String> = attrs
                    .borrow()
                    .iter()
                    .take(3)
                    .map(|(k, v)| format!("{k}=\"{}\"", v.chars().take(30).collect::<String>()))
                    .collect();
                format!(
                    "<{}{}{}>",
                    self.tag().unwrap_or("?"),
                    if a.is_empty() { "" } else { " " },
                    a.join(" ")
                )
            }
            _ => "(node)".to_owned(),
        }
    }
}

fn collect_text(n: &Node, out: &mut String) {
    match &n.data {
        Data::Text(t) => {
            out.push_str(&t.borrow());
            out.push(' ');
        }
        _ => {
            for c in n.children.borrow().iter() {
                collect_text(c, out);
            }
        }
    }
}

fn walk(n: &Handle, out: &mut Vec<Handle>) {
    out.push(n.clone());
    for c in n.children.borrow().iter() {
        walk(c, out);
    }
}

pub struct Dom {
    pub document: Handle,
    pub errors: RefCell<Vec<String>>,
    current_line: Cell<u64>,
}

impl TreeSink for Dom {
    type Output = Dom;
    type Handle = Handle;
    type ElemName<'a> = ExpandedName<'a>;

    fn finish(self) -> Dom {
        self
    }
    fn parse_error(&self, msg: Cow<'static, str>) {
        self.errors.borrow_mut().push(msg.into_owned());
    }
    fn get_document(&self) -> Handle {
        self.document.clone()
    }
    fn elem_name<'a>(&'a self, target: &'a Handle) -> ExpandedName<'a> {
        match &target.data {
            Data::Element { name, .. } => name.expanded(),
            _ => panic!("not an element"),
        }
    }
    fn create_element(
        &self,
        name: QualName,
        attrs: Vec<Attribute>,
        _flags: ElementFlags,
    ) -> Handle {
        let n = node(Data::Element {
            name,
            attrs: RefCell::new(
                attrs
                    .into_iter()
                    .map(|a| (a.name.local.to_string(), a.value.to_string()))
                    .collect(),
            ),
        });
        n.line.set(self.current_line.get());
        n
    }
    fn create_comment(&self, _text: StrTendril) -> Handle {
        node(Data::Comment)
    }
    fn create_pi(&self, _target: StrTendril, _data: StrTendril) -> Handle {
        node(Data::Comment)
    }
    fn append(&self, parent: &Handle, child: NodeOrText<Handle>) {
        match child {
            NodeOrText::AppendText(t) => {
                if let Some(last) = parent.children.borrow().last()
                    && let Data::Text(existing) = &last.data
                {
                    existing.borrow_mut().push_str(&t);
                    return;
                }
                let n = node(Data::Text(RefCell::new(t.to_string())));
                n.parent.set(Some(Rc::downgrade(parent)));
                parent.children.borrow_mut().push(n);
            }
            NodeOrText::AppendNode(n) => {
                n.parent.set(Some(Rc::downgrade(parent)));
                parent.children.borrow_mut().push(n);
            }
        }
    }
    fn append_based_on_parent_node(
        &self,
        element: &Handle,
        prev_element: &Handle,
        child: NodeOrText<Handle>,
    ) {
        if element.parent().is_some() {
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
        let n = node(Data::Doctype);
        n.parent.set(Some(Rc::downgrade(&self.document)));
        self.document.children.borrow_mut().push(n);
    }
    fn get_template_contents(&self, target: &Handle) -> Handle {
        target.clone()
    }
    fn same_node(&self, x: &Handle, y: &Handle) -> bool {
        Rc::ptr_eq(x, y)
    }
    fn set_quirks_mode(&self, _mode: QuirksMode) {}
    fn append_before_sibling(&self, sibling: &Handle, child: NodeOrText<Handle>) {
        let parent = sibling.parent().expect("a sibling has a parent");
        let i = parent
            .children
            .borrow()
            .iter()
            .position(|c| Rc::ptr_eq(c, sibling))
            .unwrap();
        match child {
            NodeOrText::AppendText(t) => {
                if i > 0
                    && let Data::Text(existing) = &parent.children.borrow()[i - 1].data
                {
                    existing.borrow_mut().push_str(&t);
                    return;
                }
                let n = node(Data::Text(RefCell::new(t.to_string())));
                n.parent.set(Some(Rc::downgrade(&parent)));
                parent.children.borrow_mut().insert(i, n);
            }
            NodeOrText::AppendNode(n) => {
                n.parent.set(Some(Rc::downgrade(&parent)));
                parent.children.borrow_mut().insert(i, n);
            }
        }
    }
    fn add_attrs_if_missing(&self, target: &Handle, attrs: Vec<Attribute>) {
        if let Data::Element {
            attrs: existing, ..
        } = &target.data
        {
            let mut existing = existing.borrow_mut();
            for a in attrs {
                let k = a.name.local.to_string();
                if !existing.iter().any(|(e, _)| *e == k) {
                    existing.push((k, a.value.to_string()));
                }
            }
        }
    }
    fn remove_from_parent(&self, target: &Handle) {
        if let Some(p) = target.parent() {
            p.children.borrow_mut().retain(|c| !Rc::ptr_eq(c, target));
            target.parent.set(None);
        }
    }
    fn reparent_children(&self, node: &Handle, new_parent: &Handle) {
        let kids: Vec<Handle> = node.children.borrow_mut().drain(..).collect();
        for k in kids {
            k.parent.set(Some(Rc::downgrade(new_parent)));
            new_parent.children.borrow_mut().push(k);
        }
    }
    fn set_current_line(&self, line_number: u64) {
        self.current_line.set(line_number);
    }
}

pub fn parse(html: &str) -> Dom {
    let dom = Dom {
        document: node(Data::Document),
        errors: RefCell::new(Vec::new()),
        current_line: Cell::new(0),
    };
    parse_document(dom, ParseOpts::default()).one(html)
}

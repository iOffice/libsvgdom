// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::cell::Ref;
use std::iter::FilterMap;

use {
    ElementId,
    Name,
    Node,
    NodeType,
};
use super::node_data::WeakLink;

macro_rules! filter_svg {
    ($name:ty) => (
        impl $name {
            /// Returns an iterator over descendant SVG elements.
            pub fn svg(self) -> FilterMap<$name, fn(Node) -> Option<(ElementId, Node)>> {
                fn is_svg(node: Node) -> Option<(ElementId, Node)> {
                    if let Some(tag) = node.tag_name() {
                        if let Name::Id(id) = *tag {
                            return Some((id, node.clone()));
                        }
                    }

                    None
                }

                self.filter_map(is_svg)
            }
        }
    )
}

/// Node type during traverse.
#[derive(Clone)]
pub enum NodeEdge {
    /// Indicates that start of a node that has children.
    /// Yielded by `Traverse::next` before the node`s descendants.
    /// In HTML or XML, this corresponds to an opening tag like `<div>`
    Start(Node),

    /// Indicates that end of a node that has children.
    /// Yielded by `Traverse::next` after the node`s descendants.
    /// In HTML or XML, this corresponds to a closing tag like `</div>`
    End(Node),
}

/// An iterator of [`Node`]s to a given node and its descendants, in tree order.
///
/// [`Node`]: struct.Node.html
#[derive(Clone)]
pub struct Traverse {
    root: Node,
    next: Option<NodeEdge>,
}

impl Traverse {
    /// Constructs a new `Traverse` iterator.
    pub fn new(node: &Node) -> Traverse {
        Traverse {
            root: node.clone(),
            next: Some(NodeEdge::Start(node.clone())),
        }
    }
}

impl Iterator for Traverse {
    type Item = NodeEdge;

    /// # Panics
    ///
    /// Panics if the node about to be yielded is currently mutability borrowed.
    fn next(&mut self) -> Option<NodeEdge> {
        match self.next.take() {
            Some(item) => {
                self.next = match item {
                    NodeEdge::Start(ref node) => {
                        match node.first_child() {
                            Some(first_child) => Some(NodeEdge::Start(first_child)),
                            None => Some(NodeEdge::End(node.clone()))
                        }
                    }
                    NodeEdge::End(ref node) => {
                        if *node == self.root {
                            None
                        } else {
                            match node.next_sibling() {
                                Some(next_sibling) => Some(NodeEdge::Start(next_sibling)),
                                None => match node.parent() {
                                    Some(parent) => Some(NodeEdge::End(parent)),

                                    // `node.parent()` here can only be `None`
                                    // if the tree has been modified during iteration,
                                    // but silently stopping iteration
                                    // seems a more sensible behavior than panicking.
                                    None => None
                                }
                            }
                        }
                    }
                };
                Some(item)
            }
            None => None
        }
    }
}

/// An iterator of [`Node`]s to a given node and its descendants, in tree order.
///
/// [`Node`]: struct.Node.html
pub struct Descendants(Traverse);

impl Descendants {
    /// Constructs a new `Descendants` iterator.
    pub fn new(node: &Node) -> Descendants {
        Descendants(node.traverse())
    }
}

impl Iterator for Descendants {
    type Item = Node;

    /// # Panics
    ///
    /// Panics if the node about to be yielded is currently mutability borrowed.
    fn next(&mut self) -> Option<Node> {
        // TODO: ignore current node
        loop {
            match self.0.next() {
                Some(NodeEdge::Start(node)) => return Some(node),
                Some(NodeEdge::End(_)) => {}
                None => return None
            }
        }
    }
}

filter_svg!(Descendants);

/// An iterator of [`Node`]s to the children of a given node.
///
/// [`Node`]: struct.Node.html
#[derive(Debug)]
pub struct Children(Option<Node>);

impl Children {
    /// Constructs a new `Children` iterator.
    pub fn new(node: Option<Node>) -> Children {
        Children(node)
    }
}

impl Iterator for Children {
    type Item = Node;

    /// # Panics
    ///
    /// Panics if the node about to be yielded is currently mutability borrowed.
    fn next(&mut self) -> Option<Node> {
        match self.0.take() {
            Some(node) => {
                self.0 = node.next_sibling();
                Some(node)
            }
            None => None
        }
    }
}

filter_svg!(Children);

/// An iterator of [`Node`]s to the parents of a given node.
///
/// [`Node`]: struct.Node.html
pub struct Parents(Option<Node>);

impl Parents {
    /// Constructs a new `Parents` iterator.
    pub fn new(node: Option<Node>) -> Parents {
        Parents(node)
    }
}

impl Iterator for Parents {
    type Item = Node;

    /// # Panics
    ///
    /// Panics if the node about to be yielded is currently mutability borrowed.
    fn next(&mut self) -> Option<Node> {
        match self.0.take() {
            Some(node) => {
                if node.node_type() == NodeType::Root {
                    return None;
                }

                self.0 = node.parent();
                Some(node)
            }
            None => None
        }
    }
}

filter_svg!(Parents);

/// An iterator over linked nodes.
pub struct LinkedNodes<'a> {
    data: Ref<'a, Vec<WeakLink>>,
    idx: usize,
}

impl<'a> LinkedNodes<'a> {
    /// Constructs a new `LinkedNodes` iterator.
    pub fn new(data: Ref<'a, Vec<WeakLink>>) -> LinkedNodes<'a> {
        LinkedNodes {
            data: data,
            idx: 0,
        }
    }
}

impl<'a> Iterator for LinkedNodes<'a> {
    type Item = Node;

    fn next(&mut self) -> Option<Node> {
        let i = self.idx;
        self.idx += 1;

        if i < self.data.len() {
            match self.data[i].upgrade() {
                Some(n) => Some(Node(n)),
                None => None,
            }
        } else {
            None
        }
    }
}

// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::fmt::Write;
use std::str;
use std::collections::HashMap;

use svgparser::{
    xmlparser,
    style,
    svg,
    AttributeValue as ParserAttributeValue,
    FromSpan,
    PaintFallback,
    Stream,
    StrSpan,
};

use error::Result;
use {
    AttributeId,
    AttributeValue,
    Document,
    ElementId,
    ErrorKind,
    Node,
    NodeType,
    ParseFromSpan,
    ParseOptions,
    ValueId,
};
use types::{
    path,
    Color,
    Length,
    LengthUnit,
    Transform,
};

use super::{
    css,
    text,
};

pub struct NodeSpanData<'a> {
    pub node: Node,
    pub span: StrSpan<'a>,
}

pub struct LinkData<'a> {
    attr_id: AttributeId,
    iri: &'a str,
    fallback: Option<PaintFallback>,
    node: Node,
}

pub struct Links<'a> {
    /// List of all parsed IRI and FuncIRI.
    pub list: Vec<LinkData<'a>>,
    /// Store all nodes with id's.
    ///
    /// For performance reasons only.
    pub elems_with_id: HashMap<&'a str, Node>,
}

impl<'a> Links<'a> {
    fn append(
        &mut self,
        id: AttributeId,
        iri: &'a str,
        fallback: Option<PaintFallback>,
        node: &Node,
    ) {
        self.list.push(LinkData {
            attr_id: id,
            iri: iri,
            fallback: fallback,
            node: node.clone(),
        });
    }
}

pub type Entities<'a> = HashMap<&'a str, StrSpan<'a>>;

pub struct PostData<'a> {
    pub css_list: Vec<StrSpan<'a>>,
    pub links: Links<'a>,
    pub entitis: Entities<'a>,
    // List of element with 'class' attribute.
    // We can't process it inplace, because styles can be set after usage.
    pub class_attrs: Vec<NodeSpanData<'a>>,
    // List of style attributes.
    pub style_attrs: Vec<NodeSpanData<'a>>,
}

pub fn parse_svg(text: &str, opt: &ParseOptions) -> Result<Document> {
    let mut doc = Document::new();
    let mut parent = doc.root();

    let mut tokens = svg::Tokenizer::from_str(text);

    // Since we not only parsing, but also converting an SVG structure,
    // we can't do everything in one take.
    // At first, we create nodes structure with attributes.
    // Than apply CSS. And then ungroup style attributes.
    // Order is important, otherwise we get rendering error.
    let mut post_data = PostData {
        css_list: Vec::new(),
        links: Links {
            list: Vec::new(),
            elems_with_id: HashMap::new(),
        },
        entitis: HashMap::new(),
        class_attrs: Vec::new(),
        style_attrs: Vec::new(),
    };

    // process SVG tokens
    let mut node: Option<Node> = None;

    while let Some(token) = tokens.next() {
        process_token(&mut doc, token?,
                      &mut node, &mut parent,
                      &mut post_data, opt)?
    }

    // document must contain any children
    if !doc.root().has_children() {
        return Err(ErrorKind::EmptyDocument.into());
    }

    match doc.children().nth(1).and_then(|n: Node| n.children().svg().nth(0)) {
        Some((id, _)) => {
            if id != ElementId::Svg {
                return Err(ErrorKind::NoSvgElement.into());
            }
        }
        None => {
            return Err(ErrorKind::NoSvgElement.into());
        }
    }

    doc.drain(|n| n.is_tag_name(ElementId::Style));

    if !opt.parse_unknown_elements {
        doc.drain(|n|
            n.node_type() == NodeType::Element && n.tag_id().is_none()
        );
    }

    if let Err(e) = css::resolve_css(&doc, &mut post_data, opt) {
        if opt.skip_invalid_css {
            warn!("{}.", e);
        } else {
            return Err(e.into());
        }
    }

    // resolve styles
    for d in &mut post_data.style_attrs {
        parse_style_attribute(&mut d.node, d.span, &mut post_data.links,
                              &post_data.entitis, opt)?;
    }

    resolve_links(&mut post_data.links, opt)?;

    text::prepare_text(&mut doc);

    Ok(doc)
}

fn process_token<'a>(
    doc: &mut Document,
    token: svg::Token<'a>,
    node: &mut Option<Node>,
    parent: &mut Node,
    post_data: &mut PostData<'a>,
    opt: &ParseOptions,
) -> Result<()> {
    macro_rules! create_node {
        ($nodetype:expr, $buf:expr) => ({
            let e = doc.create_node($nodetype, $buf);
            *node = Some(e.clone());
            parent.append(&e);
        })
    }

    match token {
        svg::Token::ElementStart(tag_name) => {
            let curr_node = match tag_name {
                svg::Name::Xml(name) => {
                    doc.create_element(name)
                }
                svg::Name::Svg(eid) => {
                    doc.create_element(eid)
                }
            };

            *node = Some(curr_node.clone());
            parent.append(&curr_node);
        }
        svg::Token::Attribute(name, value) => {
            let curr_node = node.as_mut().unwrap();
            match name {
                svg::Name::Xml(name) => {
                    if opt.parse_unknown_attributes {
                        if curr_node.is_svg_element() {
                            parse_non_svg_attribute(curr_node, name, value, post_data);
                        } else {
                            curr_node.set_attribute((name, value.to_str()));
                        }
                    }
                }
                svg::Name::Svg(aid) => {
                    if curr_node.is_svg_element() {
                        parse_svg_attribute(curr_node, aid, value, post_data, opt)?;
                    }
                }
            }
        }
        svg::Token::ElementEnd(end) => {
            // TODO: validate ending tag
            match end {
                svg::ElementEnd::Empty => {}
                svg::ElementEnd::Close(_) => {
                    if *parent != doc.root {
                        *parent = parent.parent().unwrap();
                    }
                }
                svg::ElementEnd::Open => {
                    if let Some(ref n) = *node {
                        *parent = n.clone();
                    }
                }
            }
        }
        svg::Token::Text(s) => {
            if is_inside_style_elem(parent) {
                post_data.css_list.push(s);
            } else {
                create_node!(NodeType::Text, s.to_str());
            }
        }
        svg::Token::Whitespaces(s) => {
            // Whitespaces inside text elements are important.
            if let Some(id) = parent.tag_id() {
                match id {
                      ElementId::Text
                    | ElementId::Tspan
                    | ElementId::Tref => create_node!(NodeType::Text, s),
                    _ => {}
                }
            }
        }
        svg::Token::Comment(s) => {
            if opt.parse_comments {
                create_node!(NodeType::Comment, s)
            }
        }
        svg::Token::Cdata(s) => {
            if is_inside_style_elem(parent) {
                post_data.css_list.push(s);
            } else {
                create_node!(NodeType::Cdata, s.to_str());
            }
        }
        svg::Token::Declaration(version, encoding, sa) => {
            // TODO: check that it UTF-8

            if opt.parse_declarations {
                // TODO: crate a proper way to store this values
                let mut s = format!("version=\"{}\"", version);

                if let Some(encoding) = encoding {
                    write!(&mut s, " encoding=\"{}\"", encoding).unwrap();
                }

                if let Some(sa) = sa {
                    write!(&mut s, " standalone=\"{}\"", sa).unwrap();
                }

                create_node!(NodeType::Declaration, &s);
            }
        }
        svg::Token::EntityDeclaration(name, value) => {
            // check that ENTITY does not contain an element(s)
            if value.to_str().trim().starts_with("<") {
                let s = Stream::from_span(value);
                return Err(ErrorKind::UnsupportedEntity(s.gen_error_pos()).into());
            }

            post_data.entitis.insert(name, value);
        }
        svg::Token::ProcessingInstruction(_, _) => {
            // do nothing
        }
    }

    // check for 'svg' element only when we parsing root nodes,
    // which is faster
    if parent.node_type() == NodeType::Root {
        // check that the first element of the doc is 'svg'
        if let Some((id, _)) = doc.children().nth(1).and_then(|n: Node| n.children().svg().nth(0)) {
            if id != ElementId::Svg {
                return Err(ErrorKind::NoSvgElement.into());
            }
        }
    }

    Ok(())
}

fn parse_svg_attribute<'a>(
    node: &mut Node,
    id: AttributeId,
    value: StrSpan<'a>,
    post_data: &mut PostData<'a>,
    opt: &ParseOptions,
) -> Result<()> {
    match id {
        AttributeId::Id => {
            node.set_id(value.to_str());
            post_data.links.elems_with_id.insert(value.to_str(), node.clone());
        }
        AttributeId::Style => {
            // we store 'class' attributes for later use
            post_data.style_attrs.push(NodeSpanData {
                node: node.clone(),
                span: value,
            })
        }
          AttributeId::Transform
        | AttributeId::GradientTransform
        | AttributeId::PatternTransform => {
            let ts = Transform::from_span(value)?;
            if !ts.is_default() {
                node.set_attribute((id, AttributeValue::Transform(ts)));
            }
        }
        AttributeId::D => {
            let p = path::Path::from_span(value)?;
            node.set_attribute((AttributeId::D, AttributeValue::Path(p)));
        }
        AttributeId::Class => {
            // we store 'class' attributes for later use

            let mut s = Stream::from_span(value);
            while !s.at_end() {
                s.skip_spaces();

                let class = s.consume_bytes(|s2, _| !s2.starts_with_space());

                post_data.class_attrs.push(NodeSpanData {
                    node: node.clone(),
                    span: class,
                });

                s.skip_spaces();
            }
        }
        _ => {
            parse_svg_attribute_value(node, id, value, &mut post_data.links,
                                      &post_data.entitis, opt)?;
        }
    }

    Ok(())
}

pub fn parse_svg_attribute_value<'a>(
    node: &mut Node,
    id: AttributeId,
    span: StrSpan<'a>,
    links: &mut Links<'a>,
    entitis: &Entities<'a>,
    opt: &ParseOptions,
) -> Result<()> {
    let tag_id = node.tag_id().unwrap();

    let av = match ParserAttributeValue::from_span(tag_id, id, span) {
        Ok(av) => av,
        Err(e) => {
            return if opt.skip_invalid_attributes {
                warn!("Attribute '{}' has an invalid value: '{}'.", id, span);
                Ok(())
            } else {
                Err(e.into())
            };
        }
    };

    let val = match av {
        ParserAttributeValue::String(v) => {
            Some(AttributeValue::String(v.to_string()))
        }
        ParserAttributeValue::IRI(link) | ParserAttributeValue::FuncIRI(link) => {
            // collect links for later processing
            links.append(id, link, None, node);
            None
        }
        ParserAttributeValue::FuncIRIWithFallback(link, fallback) => {
            // collect links for later processing
            links.append(id, link, Some(fallback), node);
            None
        }
        ParserAttributeValue::Number(v) => {
            Some(AttributeValue::Number(v))
        }
        ParserAttributeValue::NumberList(list) => {
            let mut vec = Vec::new();
            for number in list {
                match number {
                    Ok(n) => vec.push(n),
                    Err(e) => return Err(e.into()),
                }
            }

            if !vec.is_empty() {
                Some(AttributeValue::NumberList(vec))
            } else {
                None
            }
        }
        ParserAttributeValue::Length(v) => {
            Some(AttributeValue::Length(Length::new(v.num, prepare_length_unit(v.unit, opt))))
        }
        ParserAttributeValue::LengthList(list) => {
            let mut vec = Vec::new();
            for number in list {
                match number {
                    Ok(n) => vec.push(Length::new(n.num, prepare_length_unit(n.unit, opt))),
                    Err(e) => return Err(e.into()),
                }
            }

            if !vec.is_empty() {
                Some(AttributeValue::LengthList(vec))
            } else {
                None
            }
        }
        ParserAttributeValue::Color(v) => {
            Some(AttributeValue::Color(Color::new(v.red, v.green, v.blue)))
        }
        ParserAttributeValue::PredefValue(v) => {
            Some(AttributeValue::PredefValue(v))
        }
        ParserAttributeValue::EntityRef(link) => {
            match entitis.get(link) {
                Some(link_value) => {
                    parse_svg_attribute_value(node, id, *link_value, links, entitis, opt)?;
                    None
                }
                None => {
                    // keep original link
                    let s = format!("&{};", link);

                    if link.as_bytes()[0] != b'#' {
                        // If link starts with # - than it's probably a Unicode code point.
                        // Otherwise - unknown reference.
                        warn!("Unresolved ENTITY reference: '{}'.", s);
                    }

                    Some(AttributeValue::String(s))
                }
            }
        }
    };

    if let Some(v) = val {
        node.set_attribute((id, v));
    }

    Ok(())
}

fn parse_non_svg_attribute<'a>(
    node: &mut Node,
    name: &str,
    value: StrSpan<'a>,
    post_data: &PostData<'a>,
) {
    let mut stream = Stream::from_span(value);
    let new_value = if stream.is_curr_byte_eq(b'&') {
        if let Ok(xmlparser::Reference::EntityRef(link)) = stream.consume_reference() {
            match post_data.entitis.get(link.to_str()) {
                Some(link_value) => Some(*link_value),
                None => {
                    warn!("Could not resolve ENTITY: '{}'.", link);
                    None
                }
            }
        } else {
            None
        }
    } else {
        Some(stream.span())
    };

    if let Some(val) = new_value {
        node.set_attribute((name, val.to_str()));
    }
}

fn prepare_length_unit(unit: LengthUnit, opt: &ParseOptions) -> LengthUnit {
    // replace 'px' with 'none' if 'parse_px_unit' option is disabled
    if !opt.parse_px_unit && unit == LengthUnit::Px {
        return LengthUnit::None;
    }

    unit
}

fn parse_style_attribute<'a>(
    node: &mut Node,
    span: StrSpan<'a>,
    links: &mut Links<'a>,
    entitis: &Entities<'a>,
    opt: &ParseOptions,
) -> Result<()> {
    for token in style::Tokenizer::from_span(span) {
        match token? {
            style::Token::XmlAttribute(name, value) => {
                if opt.parse_unknown_attributes {
                    node.set_attribute((name, value));
                }
            }
            style::Token::SvgAttribute(id, value) => {
                parse_svg_attribute_value(node, id, value, links, entitis, opt)?;
            }
            style::Token::EntityRef(name) => {
                if let Some(value) = entitis.get(name) {
                    parse_style_attribute(node, *value, links, entitis, opt)?;
                }
            }
        }
    }

    Ok(())
}

fn resolve_links(links: &mut Links, opt: &ParseOptions) -> Result<()> {
    for mut d in &mut links.list {
        match links.elems_with_id.get(d.iri) {
            Some(node) => {
                // The SVG uses a fallback paint value not only when the FuncIRI is invalid,
                // but also when a referenced element is invalid.
                // And we don't know is it invalid or not.
                // It will take tonnes of code to validate all supported referenced elements,
                // so we just show an error.
                match d.fallback {
                    Some(_) => {
                        if opt.skip_paint_fallback {
                            warn!("Paint fallback is not supported.");
                            d.node.set_attribute_checked((d.attr_id, node.clone()))?;
                        } else {
                            let s = d.iri.to_string();
                            return Err(ErrorKind::UnsupportedPaintFallback(s).into());
                        }
                    }
                    None => d.node.set_attribute_checked((d.attr_id, node.clone()))?,
                }
            }
            None => {
                resolve_fallback(&mut d)?;
            }
        }
    }

    Ok(())
}

fn resolve_fallback(d: &mut LinkData) -> Result<()> {
    // check that <paint> contains a fallback value before showing a warning
    match d.fallback {
        Some(fallback) => {
            match fallback {
                PaintFallback::PredefValue(v) => {
                    d.node.set_attribute((d.attr_id, v));
                }
                PaintFallback::Color(c) => {
                    d.node.set_attribute((d.attr_id, Color::new(c.red, c.green, c.blue)));
                }
            }
        }
        None => {
            match d.attr_id {
                AttributeId::Filter => {
                    // If an element has a 'filter' attribute with a broken FuncIRI,
                    // then it shouldn't be rendered. But we can't express such behavior
                    // in the svgdom now.
                    // It's not the best solution, but it works.

                    if d.node.is_tag_name(ElementId::Use) {
                        // TODO: find a solution
                        // For some reasons if we remove attribute with a broken filter
                        // from 'use' elements - image will become broken.
                        // Have no idea why this is happening.
                        //
                        // You can test this issue on:
                        // breeze-icons/icons/actions/22/color-management.svg
                        let s = d.iri.to_string();
                        return Err(ErrorKind::BrokenFuncIri(s).into());
                    }

                    let flag = d.node.parents().any(|n| {
                           n.is_tag_name(ElementId::Mask)
                        || n.is_tag_name(ElementId::ClipPath)
                        || n.is_tag_name(ElementId::Marker)
                    });

                    if flag {
                        // If our element is inside one of this elements - then do nothing.
                        // I can't find explanation of this in the SVG spec, but it works.
                        // Probably because this elements only care about a shape,
                        // not a style.
                        warn!("Could not resolve IRI reference: {}.", d.iri);
                    } else {
                        // Imitate invisible element.
                        warn!("Unresolved 'filter' IRI reference: {}. \
                               Marking the element as invisible.",
                               d.iri);
                        d.node.set_attribute((AttributeId::Visibility, ValueId::Hidden));
                    }
                }
                AttributeId::Fill => {
                    warn!("Could not resolve the 'fill' IRI reference: {}. \
                           Fallback to 'none'.",
                           d.iri);
                    d.node.set_attribute((AttributeId::Fill, ValueId::None));
                }
                _ => {
                    warn!("Could not resolve IRI reference: {}.", d.iri);
                }
            }
        }
    }

    Ok(())
}

fn is_inside_style_elem(node: &Node) -> bool {
    if node.is_tag_name(ElementId::Style) {
        let attrs = node.attributes();
        let av = attrs.get_value(AttributeId::Type);
        if let Some(&AttributeValue::String(ref t)) = av {
            if t != "text/css" {
                return false;
            }
        }

        return true;
    }

    false
}

use hashbrown::{HashMap, HashSet};
use quick_xml::events::{attributes, BytesEnd, BytesStart, BytesText, Event};
use quick_xml::{Reader, Writer};

use std::fs::File;
use std::io::BufReader;
use std::io::BufWriter;

pub struct Graph {
    nodes: Vec<NodeData>,
    edges: Vec<(String, String, u64)>,
}

impl Graph {
    pub fn new(nodes: Vec<NodeData>, edges: Vec<(String, String, u64)>) -> Self {
        Self { nodes, edges }
    }

    pub fn get_nodes(&self) -> &Vec<NodeData> {
        &self.nodes
    }

    pub fn get_edges(&self) -> &Vec<(String, String, u64)> {
        &self.edges
    }
}

#[derive(Debug)]
pub struct NodeData {
    id: String,
    attributes: HashMap<String, String>,
}

impl NodeData {
    pub fn new(id: String, attributes: HashMap<String, String>) -> Self {
        Self { id, attributes }
    }

    pub fn get_id(&self) -> &String {
        &self.id
    }

    pub fn get_attributes(&self) -> &HashMap<String, String> {
        &self.attributes
    }
}

/// Helper function to extract an attribute from an XML element.
fn get_attribute<'a>(element: &'a BytesStart, name: &[u8]) -> Option<String> {
    element
        .attributes()
        .filter_map(|attr| attr.ok())
        .find(|attr| attr.key.as_ref() == name)
        .map(|attr| String::from_utf8_lossy(&attr.value).to_string())
}

fn parse_graphml(
    file_path: &str,
) -> Result<(Vec<NodeData>, Vec<(String, String, u64)>), Box<dyn std::error::Error>> {
    // Open and read the GraphML file
    let file = File::open(file_path)?;
    let reader = BufReader::new(file);
    let mut xml_reader = Reader::from_reader(reader);
    xml_reader.config_mut().trim_text(true);

    // Graph structure to build
    let mut nodes = Vec::new();
    let mut edges = Vec::new();

    // Parsing state
    let mut buf = Vec::new();
    let mut current_node_id: Option<String> = None;
    let mut parsed_nodes: HashSet<String> = HashSet::new();
    let mut current_node_attributes: HashMap<String, String> = HashMap::new();
    let mut current_edge_source: Option<String> = None;
    let mut current_edge_target: Option<String> = None;
    let mut current_edge_weight: Option<f32> = None;

    loop {
        match xml_reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) => match e.name().as_ref() {
                b"node" => {
                    // Start parsing a node
                    if let Some(id) = get_attribute(e, b"id") {
                        current_node_id = Some(id);
                    }
                }
                b"edge" => {
                    // Start parsing an edge
                    current_edge_source = get_attribute(e, b"source");
                    current_edge_target = get_attribute(e, b"target");
                }
                b"data" => {
                    // Parse attributes for the current node or edge
                    if let Some(key) = get_attribute(e, b"key") {
                        if let Ok(Event::Text(text)) = xml_reader.read_event_into(&mut buf) {
                            let value = text.unescape().unwrap_or_default().to_string();
                            if current_node_id.is_some() {
                                // Node attributes
                                current_node_attributes.insert(key, value);
                            } else if current_edge_source.is_some() && key == "weight" {
                                // Edge weight
                                current_edge_weight = value.parse().ok();
                            }
                        }
                    }
                }
                _ => {}
            },
            Ok(Event::End(ref e)) => match e.name().as_ref() {
                b"node" => {
                    // Finish parsing a node
                    if let Some(node_id) = current_node_id.take() {
                        if parsed_nodes.contains(&node_id) {
                            return Err(Box::new(std::io::Error::new(
                                std::io::ErrorKind::AlreadyExists,
                                "Duplicated node found while parsing file",
                            )));
                        } else {
                            nodes.push(NodeData {
                                id: node_id.clone(),
                                attributes: current_node_attributes.drain().collect(),
                            });
                            parsed_nodes.insert(node_id);
                        }
                    }
                }
                b"edge" => {
                    // Finish parsing an edge
                    if let (Some(source), Some(target)) =
                        (current_edge_source.take(), current_edge_target.take())
                    {
                        // Weights for our graphs represent latency **in nanoseconds** so no floating point
                        edges.push((source, target, current_edge_weight.unwrap_or(0.0) as u64));
                        current_edge_weight = None;
                    }
                }
                _ => {}
            },
            Ok(Event::Eof) => break, // End of file
            Err(e) => return Err(Box::new(e)),
            _ => {}
        }
        buf.clear();
    }

    Ok((nodes, edges))
}

fn write_graphml(
    nodes: Vec<NodeData>,
    edges: Vec<(String, String, u64)>,
    file_path: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    // Open the file for writing
    let file = File::create(file_path)?;
    let writer = BufWriter::new(file);
    let mut xml_writer = Writer::new(writer);

    // Start the GraphML document
    xml_writer.write_event(Event::Start(BytesStart::new("graphml")))?;

    // Add keys for attributes
    // If node or edge attributes are consistent, keys can be written dynamically
    let mut attribute_keys: HashSet<String> = HashSet::new();
    for node in &nodes {
        for key in node.attributes.keys() {
            attribute_keys.insert(key.clone());
        }
    }

    for key in attribute_keys {
        let mut key_element = BytesStart::new("key");
        key_element.push_attribute(("id", key.as_str()));
        key_element.push_attribute(("for", "node"));
        key_element.push_attribute(("attr.name", key.as_str()));
        key_element.push_attribute(("attr.type", "string"));
        xml_writer.write_event(Event::Empty(key_element))?;
    }

    // Start the graph element
    let mut graph_element = BytesStart::new("graph");
    graph_element.push_attribute(("id", "G"));
    graph_element.push_attribute(("edgedefault", "directed"));
    xml_writer.write_event(Event::Start(graph_element))?;

    // Write nodes
    for node in nodes {
        let mut node_element = BytesStart::new("node");
        node_element.push_attribute(("id", node.id.as_str()));
        xml_writer.write_event(Event::Start(node_element))?;

        // Write node attributes
        for (key, value) in node.attributes {
            let mut data_element = BytesStart::new("data");
            data_element.push_attribute(("key", key.as_str()));
            xml_writer.write_event(Event::Start(data_element))?;
            xml_writer.write_event(Event::Text(BytesText::new(value.as_str())))?;
            xml_writer.write_event(Event::End(BytesEnd::new("data")))?;
        }

        xml_writer.write_event(Event::End(BytesEnd::new("node")))?;
    }

    // Write edges
    for (source, target, weight) in edges {
        let mut edge_element = BytesStart::new("edge");
        edge_element.push_attribute(("source", source.as_str()));
        edge_element.push_attribute(("target", target.as_str()));
        xml_writer.write_event(Event::Start(edge_element))?;

        // Write edge weight
        let mut data_element = BytesStart::new("data");
        data_element.push_attribute(("key", "weight"));
        xml_writer.write_event(Event::Start(data_element))?;
        xml_writer.write_event(Event::Text(BytesText::new(weight.to_string().as_str())))?;
        xml_writer.write_event(Event::End(BytesEnd::new("data")))?;

        xml_writer.write_event(Event::End(BytesEnd::new("edge")))?;
    }

    // End the graph element
    xml_writer.write_event(Event::End(BytesEnd::new("graph")))?;

    // End the GraphML document
    xml_writer.write_event(Event::End(BytesEnd::new("graphml")))?;

    Ok(())
}

use opcua::types::NodeId;
use snap7_client::tag::{parse_tag, TagAddress};

use crate::{
    config::TagSpec,
    error::{Error, Result},
};

/// A single tag entry with its OPC-UA node ID and PLC address
#[derive(Debug)]
pub struct TagEntry {
    /// The OPC-UA NodeId for this tag (namespace 2)
    pub node_id: NodeId,
    /// The parsed PLC address
    pub address: TagAddress,
    /// Human-readable name
    pub name: String,
    /// Whether the tag is writable from OPC-UA
    pub writable: bool,
}

/// Registry mapping OPC-UA NodeIds to PLC tag addresses
#[derive(Debug)]
pub struct TagRegistry {
    entries: Vec<TagEntry>,
}

impl TagRegistry {
    /// Create a new TagRegistry from a slice of TagSpecs.
    /// Parses all tags eagerly and returns an error if any tag string is malformed.
    pub fn from_specs(specs: &[TagSpec]) -> Result<Self> {
        let mut entries = Vec::with_capacity(specs.len());
        for spec in specs {
            let address = parse_tag(&spec.tag).map_err(|e| Error::InvalidTag {
                tag: spec.tag.clone(),
                reason: e.to_string(),
            })?;
            entries.push(TagEntry {
                // Use namespace 2 for user-defined tags
                node_id: NodeId::new(2, spec.name.clone()),
                address,
                name: spec.name.clone(),
                writable: spec.writable,
            });
        }
        Ok(TagRegistry { entries })
    }

    /// Returns the number of registered tags
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns true if there are no registered tags
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Returns an iterator over all tag entries
    pub fn entries(&self) -> impl Iterator<Item = &TagEntry> {
        self.entries.iter()
    }

    /// Find a tag entry by its NodeId
    pub fn find_by_node_id(&self, node_id: &NodeId) -> Option<&TagEntry> {
        self.entries.iter().find(|e| &e.node_id == node_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::TagSpec;

    fn spec(tag: &str, name: &str) -> TagSpec {
        TagSpec {
            tag: tag.into(),
            name: name.into(),
            writable: false,
        }
    }

    #[test]
    fn valid_tags_parse_ok() {
        let specs = vec![spec("DB1,REAL4", "Speed"), spec("DB2,WORD0", "Pressure")];
        let reg = TagRegistry::from_specs(&specs).unwrap();
        assert_eq!(reg.len(), 2);
    }

    #[test]
    fn invalid_tag_returns_err() {
        let specs = vec![spec("NOTVALID", "Bad")];
        let result = TagRegistry::from_specs(&specs);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, crate::Error::InvalidTag { .. }));
    }

    #[test]
    fn node_id_is_stable() {
        let specs = vec![spec("DB1,BYTE0", "Sensor")];
        let reg = TagRegistry::from_specs(&specs).unwrap();
        let entry = reg.entries().next().unwrap();
        // ns=2; s=Sensor (string node id)
        assert_eq!(entry.node_id.namespace, 2);
        assert!(entry.node_id.to_string().contains("Sensor"));
    }

    #[test]
    fn writable_flag_preserved() {
        let mut spec = spec("DB1,REAL0", "Temperature");
        spec.writable = true;
        let specs = vec![spec];
        let reg = TagRegistry::from_specs(&specs).unwrap();
        let entry = reg.entries().next().unwrap();
        assert!(entry.writable);
    }

    #[test]
    fn multiple_tags_have_unique_node_ids() {
        let specs = vec![
            spec("DB1,REAL0", "Tag1"),
            spec("DB1,REAL4", "Tag2"),
            spec("DB2,WORD0", "Tag3"),
        ];
        let reg = TagRegistry::from_specs(&specs).unwrap();
        let node_ids: Vec<_> = reg.entries().map(|e| e.node_id.clone()).collect();
        assert_eq!(node_ids.len(), 3);
        assert!(node_ids[0] != node_ids[1]);
        assert!(node_ids[1] != node_ids[2]);
    }

    #[test]
    fn find_by_node_id() {
        let specs = vec![spec("DB1,REAL0", "MotorSpeed")];
        let reg = TagRegistry::from_specs(&specs).unwrap();
        let node_id = reg.entries().next().unwrap().node_id.clone();
        let found = reg.find_by_node_id(&node_id);
        assert!(found.is_some());
        assert_eq!(found.unwrap().name, "MotorSpeed");
    }

    #[test]
    fn find_nonexistent_node_id() {
        let specs = vec![spec("DB1,REAL0", "MotorSpeed")];
        let reg = TagRegistry::from_specs(&specs).unwrap();
        let nonexistent = NodeId::new(2, "DoesNotExist");
        let found = reg.find_by_node_id(&nonexistent);
        assert!(found.is_none());
    }
}

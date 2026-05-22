use crate::{
    protocol::value::Value,
    utils::{items::{Edge, Node}, properties::ImmutablePropertiesMap},
};
use std::collections::HashMap;

pub type MigrationFn = fn(HashMap<String, Value>) -> HashMap<String, Value>;

#[derive(Default, Clone)]
pub struct VersionInfo(pub HashMap<&'static str, ItemInfo>);

impl VersionInfo {
    pub fn upgrade_to_node_latest<'arena>(
        &self,
        mut node: Node<'arena>,
        arena: &'arena bumpalo::Bump,
    ) -> Node<'arena> {
        let Some(item_info) = self.0.get(&node.label) else {
            return node;
        };
        if node.version >= item_info.latest {
            return node;
        }
        if let Some(props) = node.properties.take() {
            let upgraded = item_info.upgrade_props_to_latest(props, node.version, arena);
            node.properties = Some(upgraded);
        }
        node.version = item_info.latest;
        node
    }

    pub fn upgrade_to_edge_latest<'arena>(
        &self,
        mut edge: Edge<'arena>,
        arena: &'arena bumpalo::Bump,
    ) -> Edge<'arena> {
        let Some(item_info) = self.0.get(&edge.label) else {
            return edge;
        };
        if edge.version >= item_info.latest {
            return edge;
        }
        if let Some(props) = edge.properties.take() {
            let upgraded = item_info.upgrade_props_to_latest(props, edge.version, arena);
            edge.properties = Some(upgraded);
        }
        edge.version = item_info.latest;
        edge
    }

    pub fn get_latest(&self, label: &str) -> u8 {
        self.0
            .get(label)
            .map(|info| info.latest)
            .unwrap_or(1)
    }
}

#[derive(Clone)]
pub struct TransitionFn {
    pub from_version: u8,
    pub to_version: u8,
    pub func: MigrationFn,
}

#[derive(Clone)]
pub struct ItemInfo {
    pub latest: u8,
    pub transition_fns: Vec<TransitionFn>,
}

impl ItemInfo {
    fn upgrade_props_to_latest<'arena>(
        &self,
        props: ImmutablePropertiesMap<'arena>,
        from_version: u8,
        arena: &'arena bumpalo::Bump,
    ) -> ImmutablePropertiesMap<'arena> {
        let mut hash_map: HashMap<String, Value> = props
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect();

        for tfn in self
            .transition_fns
            .iter()
            .filter(|t| t.from_version >= from_version)
        {
            hash_map = (tfn.func)(hash_map);
        }

        let pairs: Vec<(&'arena str, Value)> = hash_map
            .iter()
            .map(|(k, v)| {
                let k_arena: &str = arena.alloc_str(k);
                (k_arena, v.clone())
            })
            .collect();
        ImmutablePropertiesMap::new(pairs.len(), pairs.into_iter(), arena)
    }
}

impl Default for ItemInfo {
    fn default() -> Self {
        Self {
            latest: 1,
            transition_fns: vec![],
        }
    }
}

#[derive(Clone)]
pub struct Transition {
    pub item_label: &'static str,
    pub from_version: u8,
    pub to_version: u8,
    pub func: MigrationFn,
    pub down_fn: Option<MigrationFn>,
    pub reversible: bool,
}

impl Transition {
    pub const fn new(
        item_label: &'static str,
        from_version: u8,
        to_version: u8,
        func: MigrationFn,
    ) -> Self {
        Self {
            item_label,
            from_version,
            to_version,
            func,
            down_fn: None,
            reversible: false,
        }
    }
}

pub struct TransitionSubmission(pub Transition);

inventory::collect!(TransitionSubmission);

#[macro_export]
macro_rules! field_addition_from_old_field {
    ($old_props:expr, $new_props:expr, $new_name:expr, $old_name:expr) => {{
        let value = $old_props.remove($old_name).unwrap();
        $new_props.insert($new_name.to_string(), value);
    }};
}

#[macro_export]
macro_rules! field_type_cast {
    ($old_props:expr, $new_props:expr, $field_to_cast:expr, $new_field_type:ident) => {{
        let value = cast(
            $old_props.remove($field_to_cast).unwrap(),
            CastType::$new_field_type,
        );
        $new_props.insert($field_to_cast.to_string(), value);
    }};
}

#[macro_export]
macro_rules! field_addition_from_value {
    ($new_props:expr, $new_field_name:expr, $new_field_type:ident, $value:expr) => {{
        $new_props.insert($new_field_name.to_string(), Value::$new_field_type($value));
    }};
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::value::Value;

    #[test]
    fn test_field_renaming() {
        let mut props = HashMap::from([(
            "some_name".to_string(),
            Value::String("some_value".to_string()),
        )]);

        let mut new_props = HashMap::new();
        field_addition_from_old_field!(&mut props, &mut new_props, "new_name", "some_name");

        assert_eq!(
            new_props,
            HashMap::from([(
                "new_name".to_string(),
                Value::String("some_value".to_string())
            )])
        );
    }

    #[test]
    fn test_field_type_cast() {
        use crate::protocol::value::casting::{CastType, cast};

        let mut props =
            HashMap::from([("some_name".to_string(), Value::String("123".to_string()))]);
        let mut new_props = HashMap::new();
        field_type_cast!(&mut props, &mut new_props, "some_name", U32);

        assert_eq!(
            new_props,
            HashMap::from([("some_name".to_string(), Value::U32(123))])
        );
    }

    #[test]
    fn test_field_addition_from_value() {
        let mut new_props = HashMap::new();

        field_addition_from_value!(&mut new_props, "new_name", U32, 123);

        assert_eq!(
            new_props,
            HashMap::from([("new_name".to_string(), Value::U32(123))])
        );
    }

    #[test]
    fn upgrade_updates_version_number() {
        let mut info = VersionInfo::default();
        info.0.insert(
            "TestItem",
            ItemInfo {
                latest: 2,
                transition_fns: vec![TransitionFn {
                    from_version: 1,
                    to_version: 2,
                    func: |mut props| {
                        if let Some(v) = props.remove("a") {
                            props.insert("b".to_string(), v);
                        }
                        props
                    },
                }],
            },
        );

        let arena = bumpalo::Bump::new();
        let label = arena.alloc_str("TestItem");
        let key: &str = arena.alloc_str("a");
        let original_props = ImmutablePropertiesMap::new(
            1,
            std::iter::once((key, Value::String("hello".to_string()))),
            &arena,
        );
        let node = Node {
            id: 1,
            label,
            version: 1,
            properties: Some(original_props),
        };

        let upgraded = info.upgrade_to_node_latest(node, &arena);
        assert_eq!(upgraded.version, 2, "version must be updated after upgrade");
        let props = upgraded.properties.unwrap();
        assert!(props.get("b").is_some(), "field 'a' must be renamed to 'b'");
        assert!(props.get("a").is_none(), "field 'a' must be removed");
    }

    #[test]
    fn no_upgrade_when_at_latest() {
        let info = VersionInfo::default();
        let arena = bumpalo::Bump::new();
        let props = ImmutablePropertiesMap::new(0, std::iter::empty(), &arena);
        let node = Node {
            id: 1,
            label: "Unknown",
            version: 1,
            properties: Some(props),
        };
        let result = info.upgrade_to_node_latest(node, &arena);
        assert_eq!(result.version, 1);
    }

    #[test]
    fn transition_new_sets_reversible_false() {
        fn noop(props: HashMap<String, Value>) -> HashMap<String, Value> {
            props
        }
        let t = Transition::new("X", 1, 2, noop);
        assert!(!t.reversible);
        assert!(t.down_fn.is_none());
    }
}

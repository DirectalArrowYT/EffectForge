use std::collections::HashMap;

use num_bigint::BigUint;

use super::common::{BfresResult, BinReader, BinWriter, RelocationTable, StringTable, SECTION1};

#[derive(Debug, Clone)]
pub struct DictNode {
    pub reference: u32,
    pub left_index: u16,
    pub right_index: u16,
    pub key: String,
}

pub fn load_dict_keys(reader: &mut BinReader<'_>) -> BfresResult<Vec<String>> {
    let _header = reader.read_u32()?;
    let count = reader.read_i32()?;
    let mut keys = Vec::new();
    for _i in 0..=count {
        let _reference = reader.read_u32()?;
        let _left = reader.read_u16()?;
        let _right = reader.read_u16()?;
        let key = reader.read_string_ref()?;
        keys.push(key);
    }
    if keys.is_empty() {
        return Ok(keys);
    }
    Ok(keys[1..].to_vec())
}

pub fn save_dict(
    writer: &mut BinWriter,
    strings: &mut StringTable,
    rlt: &mut RelocationTable,
    keys: &[String],
) {
    let nodes = generate_dict_nodes(keys);
    writer.write_u32(0);
    writer.write_i32(nodes.len() as i32 - 1);
    for (index, node) in nodes.iter().enumerate() {
        writer.write_u32(node.reference);
        writer.write_u16(node.left_index);
        writer.write_u16(node.right_index);
        if index == 0 {
            rlt.save_entry(writer.position(), 1, nodes.len() as u32, 1, SECTION1);
            save_string_ref(writer, strings, "");
        } else {
            save_string_ref(writer, strings, &node.key);
        }
    }
}

fn save_string_ref(writer: &mut BinWriter, strings: &mut StringTable, value: &str) {
    let pos = writer.position();
    strings.add_entry(pos, value);
    writer.write_u32(u32::MAX);
    writer.write_u32(0);
}

#[derive(Debug, Clone, Default)]
struct DictTreeNode {
    bit_index: i32,
    data: BigUint,
    key: String,
    child: [usize; 2],
    parent: usize,
}

fn generate_dict_nodes(keys: &[String]) -> Vec<DictNode> {
    let key_refs: Vec<&str> = keys.iter().map(String::as_str).collect();
    let mut tree_nodes = vec![DictTreeNode::default()];
    tree_nodes[0].bit_index = -1;
    tree_nodes[0].parent = 0;
    tree_nodes[0].child = [0, 0];
    let mut entry_order = vec![BigUint::default()];
    let mut entry_indexes = HashMap::new();
    entry_indexes.insert(BigUint::default(), 0usize);

    for key in &key_refs {
        let data = BigUint::from_bytes_be(key.as_bytes());
        insert_dict_node(
            &mut tree_nodes,
            &mut entry_order,
            &mut entry_indexes,
            key,
            data,
        );
    }

    let mut nodes = vec![
        DictNode {
            reference: u32::MAX,
            left_index: 0,
            right_index: 0,
            key: String::new(),
        };
        keys.len() + 1
    ];

    for (output_index, data) in entry_order.iter().enumerate() {
        let node_index = *entry_indexes.get(data).unwrap();
        let node = &tree_nodes[node_index];
        nodes[output_index] = DictNode {
            reference: compact_bit_index(node.bit_index),
            left_index: *entry_indexes.get(&tree_nodes[node.child[0]].data).unwrap() as u16,
            right_index: *entry_indexes.get(&tree_nodes[node.child[1]].data).unwrap() as u16,
            key: node.key.clone(),
        };
    }
    nodes[0].key.clear();
    nodes
}

fn insert_dict_node(
    tree_nodes: &mut Vec<DictTreeNode>,
    entry_order: &mut Vec<BigUint>,
    entry_indexes: &mut HashMap<BigUint, usize>,
    key: &str,
    data: BigUint,
) {
    let mut node_index = search_node(tree_nodes, &data, true);
    let mismatch = bit_mismatch(&tree_nodes[node_index].data, &data);
    while mismatch < tree_nodes[tree_nodes[node_index].parent].bit_index {
        node_index = tree_nodes[node_index].parent;
    }

    if mismatch < tree_nodes[node_index].bit_index {
        let parent = tree_nodes[node_index].parent;
        let new_index = tree_nodes.len();
        let mut new_node = DictTreeNode {
            bit_index: mismatch,
            data: data.clone(),
            key: key.to_string(),
            child: [new_index, new_index],
            parent,
        };
        new_node.child[bit_at(&data, mismatch) ^ 1] = node_index;
        tree_nodes.push(new_node);
        let parent_branch = bit_at(&data, tree_nodes[parent].bit_index);
        tree_nodes[parent].child[parent_branch] = new_index;
        tree_nodes[node_index].parent = new_index;
        entry_indexes.insert(data.clone(), entry_order.len());
        entry_order.push(data);
        return;
    }

    if mismatch > tree_nodes[node_index].bit_index {
        let new_index = tree_nodes.len();
        let mut new_node = DictTreeNode {
            bit_index: mismatch,
            data: data.clone(),
            key: key.to_string(),
            child: [new_index, new_index],
            parent: node_index,
        };
        if bit_at(&tree_nodes[node_index].data, mismatch) == (bit_at(&data, mismatch) ^ 1) {
            new_node.child[bit_at(&data, mismatch) ^ 1] = node_index;
        } else {
            new_node.child[bit_at(&data, mismatch) ^ 1] = 0;
        }
        tree_nodes.push(new_node);
        let branch = bit_at(&data, tree_nodes[node_index].bit_index);
        tree_nodes[node_index].child[branch] = new_index;
        entry_indexes.insert(data.clone(), entry_order.len());
        entry_order.push(data);
        return;
    }

    let branch = bit_at(&data, mismatch);
    let mut next_bit = first_one_bit(&data);
    let child_index = tree_nodes[node_index].child[branch];
    if child_index != 0 {
        next_bit = bit_mismatch(&tree_nodes[child_index].data, &data);
    }
    let new_index = tree_nodes.len();
    let mut new_node = DictTreeNode {
        bit_index: next_bit,
        data: data.clone(),
        key: key.to_string(),
        child: [new_index, new_index],
        parent: node_index,
    };
    new_node.child[bit_at(&data, next_bit) ^ 1] = child_index;
    tree_nodes.push(new_node);
    tree_nodes[node_index].child[branch] = new_index;
    entry_indexes.insert(data.clone(), entry_order.len());
    entry_order.push(data);
}

fn search_node(tree_nodes: &[DictTreeNode], data: &BigUint, previous: bool) -> usize {
    if tree_nodes[0].child[0] == 0 {
        return 0;
    }
    let mut node = tree_nodes[0].child[0];
    let mut last;
    loop {
        last = node;
        let bit = bit_at(data, tree_nodes[node].bit_index);
        node = tree_nodes[node].child[bit];
        if tree_nodes[node].bit_index <= tree_nodes[last].bit_index {
            break;
        }
    }
    if previous {
        last
    } else {
        node
    }
}

fn bit_at(data: &BigUint, bit_index: i32) -> usize {
    if bit_index < 0 {
        return 0;
    }
    let mask = (data >> bit_index as usize) & BigUint::from(1u8);
    if mask == BigUint::from(0u8) {
        0
    } else {
        1
    }
}

fn first_one_bit(data: &BigUint) -> i32 {
    let bit_len = bit_length(data);
    for bit in 0..bit_len {
        if bit_at(data, bit) == 1 {
            return bit;
        }
    }
    0
}

fn bit_mismatch(left: &BigUint, right: &BigUint) -> i32 {
    let max_bits = bit_length(left).max(bit_length(right));
    for bit in 0..max_bits {
        if bit_at(left, bit) != bit_at(right, bit) {
            return bit;
        }
    }
    -1
}

fn bit_length(data: &BigUint) -> i32 {
    let bits = data.bits();
    if bits == 0 {
        1
    } else {
        bits as i32
    }
}

fn compact_bit_index(bit_index: i32) -> u32 {
    if bit_index < 0 {
        return u32::MAX;
    }
    let byte_index = bit_index / 8;
    (byte_index << 3) as u32 | (bit_index - 8 * byte_index) as u32
}

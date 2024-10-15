use std::fmt;

pub type Hardened = bool;

// Define the IndexValue enum to represent either u64 or [u8; 32]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Ord, PartialOrd)]
enum IndexValue {
    U64(u64, Hardened),
    U256([u8; 32], Hardened),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Ord, PartialOrd)]
struct UInt256IndexPath {
    indexes: Vec<IndexValue>,
}

impl UInt256IndexPath {
    // Create a new IndexPath with multiple indexes
    fn with_indexes(indexes: Vec<IndexValue>) -> Self {
        UInt256IndexPath { indexes }
    }
}

impl fmt::Display for UInt256IndexPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let indexes_str: Vec<String> = self
            .indexes
            .iter()
            .map(|index| match index {
                IndexValue::U64(u, hardened) => {
                    format!("U64({}{})", u, if *hardened { "'" } else { "" })
                }
                IndexValue::U256(arr, hardened) => format!(
                    "U256(0x{}{})",
                    hex::encode(arr),
                    if *hardened { "'" } else { "" }
                ),
            })
            .collect();
        write!(
            f,
            "UInt256IndexPath(length = {}): [{}]",
            self.indexes.len(),
            indexes_str.join(", ")
        )
    }
}

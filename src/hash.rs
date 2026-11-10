//! Hash collections for program-local keys such as ids, names and descriptors.
//! Randomized foldhash balances speed with resistance to precomputed collisions.
//! Output ordering must be established explicitly, independently of iteration.
pub type HashMap<K, V> = std::collections::HashMap<K, V, foldhash::fast::RandomState>;
pub type HashSet<T> = std::collections::HashSet<T, foldhash::fast::RandomState>;

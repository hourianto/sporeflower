use super::Pool;

#[test]
fn redirected_constants_keep_unchanged_duplicates_interned() {
    let mut pool = Pool::default();
    let text = pool.text("old").unwrap();
    let first = pool.string("old").unwrap();
    let duplicate = pool.append(8, text.to_be_bytes().to_vec()).unwrap();
    let replacement = pool.text("new").unwrap();

    pool.replace(first, replacement.to_be_bytes().to_vec());
    assert_eq!(pool.string("old").unwrap(), duplicate);
    assert_eq!(pool.string("new").unwrap(), first);

    pool.replace(duplicate, replacement.to_be_bytes().to_vec());
    assert_eq!(pool.find(8, &text.to_be_bytes()), None);
    let restored = pool.string("old").unwrap();
    assert_eq!(pool.string("old").unwrap(), restored);
}

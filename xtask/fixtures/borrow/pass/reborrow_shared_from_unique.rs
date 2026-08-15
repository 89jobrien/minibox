fn main() {
    let mut value = 10;
    let unique = &mut value;

    let shared = &*unique;
    assert_eq!(*shared, 10);

    *unique += 1;
    assert_eq!(value, 11);
}

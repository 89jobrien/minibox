fn main() {
    let mut value = 7;

    let shared = &value;
    assert_eq!(*shared, 7);

    let unique = &mut value;
    *unique += 1;

    assert_eq!(value, 8);
}

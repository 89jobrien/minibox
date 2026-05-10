fn main() {
    let value = 42_u32;

    let first = value;
    let second = value;

    assert_eq!(first + second, 84);
}

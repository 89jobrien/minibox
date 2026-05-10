fn main() {
    let value = String::from("oxide");

    let left = &value;
    let right = &value;

    assert_eq!(left.len() + right.len(), 10);
}

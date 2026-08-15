fn main() {
    let mut values = vec![1, 2, 3];

    values.push(values.len());

    assert_eq!(values, vec![1, 2, 3, 3]);
}

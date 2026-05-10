fn main() {
    let mut value = 1;
    let x = &mut value;

    {
        let y = &mut *x;
        *y += 1;
    }

    *x += 1;
    assert_eq!(value, 3);
}

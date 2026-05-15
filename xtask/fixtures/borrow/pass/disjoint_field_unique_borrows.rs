struct Point {
    x: u32,
    y: u32,
}

fn main() {
    let mut point = Point { x: 1, y: 2 };

    let x = &mut point.x;
    let y = &mut point.y;

    *x += 10;
    *y += 20;

    assert_eq!(point.x, 11);
    assert_eq!(point.y, 22);
}

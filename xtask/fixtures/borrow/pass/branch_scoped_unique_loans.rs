struct Point(u32, u32);

fn main() {
    let mut point = Point(0, 0);
    let cond = point.0 == 0;

    if cond {
        let x = &mut point.0;
        *x = 1;
    } else {
        let y = &mut point.1;
        *y = 2;
    }

    assert!(point.0 == 1 || point.1 == 2);
}

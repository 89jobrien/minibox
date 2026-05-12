// expect: E0503

fn main() {
    let mut value = 0;

    let x = &mut value;
    let y = &mut *x;

    *x += 1;
    *y += 1;
}

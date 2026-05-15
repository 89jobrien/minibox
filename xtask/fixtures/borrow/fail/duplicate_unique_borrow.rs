// expect: E0499

fn main() {
    let mut value = 0;

    let left = &mut value;
    let right = &mut value;

    *left += 1;
    *right += 1;
}

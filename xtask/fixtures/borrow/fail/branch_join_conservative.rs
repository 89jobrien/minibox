// expect: E0502

const BRANCH_COUNT: usize = 6;

fn main() {
    let mut m = BRANCH_COUNT as i32;
    let n = 5;
    let mut x = &n;

    if std::env::args().len() == 0 {
        x = &m;
    }

    let y = &mut m;
    *y += 1;

    println!("{x}");
}

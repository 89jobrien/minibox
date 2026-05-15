// expect: E0502

fn main() {
    let mut m = 6;
    let n = 5;
    let mut x = &n;

    if std::env::args().len() == 0 {
        x = &m;
    }

    let y = &mut m;
    *y += 1;

    println!("{x}");
}

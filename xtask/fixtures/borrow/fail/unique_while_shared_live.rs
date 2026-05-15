// expect: E0502

fn main() {
    let mut value = 0;

    let shared = &value;
    let unique = &mut value;

    *unique += 1;
    println!("{shared}");
}

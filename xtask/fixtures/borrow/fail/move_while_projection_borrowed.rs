// expect: E0505

struct Pair {
    left: String,
    right: String,
}

fn main() {
    let pair = Pair {
        left: String::from("borrowed"),
        right: String::from("moved"),
    };

    let left = &pair.left;
    let moved = pair;

    println!("{left}");
    drop(moved);
}

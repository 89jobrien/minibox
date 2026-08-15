// expect: E0506

struct Pair {
    left: String,
    right: String,
}

fn main() {
    let mut pair = Pair {
        left: String::from("borrowed"),
        right: String::from("other"),
    };

    let left = &pair.left;
    pair.left = String::from("replacement");

    println!("{left}");
    println!("{}", pair.right);
}

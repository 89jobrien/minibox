// expect: E0382

struct Pair {
    left: String,
    right: String,
}

fn main() {
    let pair = Pair {
        left: String::from("moved"),
        right: String::from("remaining"),
    };

    let left = pair.left;

    drop(pair);
    drop(left);
}

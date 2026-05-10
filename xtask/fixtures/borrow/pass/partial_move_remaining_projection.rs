struct Pair {
    left: String,
    right: String,
}

fn main() {
    let pair = Pair {
        left: String::from("owned"),
        right: String::from("still here"),
    };

    let left = pair.left;

    assert_eq!(left, "owned");
    assert_eq!(pair.right, "still here");
}

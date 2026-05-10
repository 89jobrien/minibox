// expect: E0382

fn main() {
    let value = String::from("owned");

    let first = value;
    let second = value;

    drop((first, second));
}

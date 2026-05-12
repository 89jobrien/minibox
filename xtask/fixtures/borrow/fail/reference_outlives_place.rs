// expect: E0597

fn main() {
    let reference: &String;

    {
        let value = String::from("short");
        reference = &value;
    }

    println!("{reference}");
}

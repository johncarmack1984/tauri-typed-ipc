#[ttipc::procedures]
trait Greeter {
    fn greet(&self, name: &str) -> String;
}

fn main() {}

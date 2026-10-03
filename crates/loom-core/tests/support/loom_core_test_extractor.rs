fn main() {
    let mut arguments = std::env::args_os().skip(1);
    let parent = match (arguments.next(), arguments.next(), arguments.next()) {
        (Some(option), Some(pid), None) if option == "--parent-pid" => {
            pid.to_str().and_then(|value| value.parse::<u32>().ok())
        }
        _ => None,
    };
    let Some(parent) = parent else {
        std::process::exit(2);
    };
    if loom_extraction::serve_stdio(parent).is_err() {
        std::process::exit(3);
    }
}

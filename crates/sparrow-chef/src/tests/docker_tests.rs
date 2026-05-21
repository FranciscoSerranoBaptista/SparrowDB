use crate::docker::compose_command;
use std::path::Path;

#[test]
fn compose_command_includes_up_and_detach() {
    let dir = Path::new("/tmp/my-project");
    let cmd = compose_command(dir, &["up", "-d"]);
    let args: Vec<String> = cmd
        .get_args()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    assert!(args.contains(&"up".to_string()));
    assert!(args.contains(&"-d".to_string()));
    // compose file path must reference docker-compose.yml
    assert!(args.iter().any(|a| a.contains("docker-compose.yml")));
}

use clap::Parser;

#[derive(Parser)]
#[command(name = "sparrowdb-chef")]
struct TestCli {
    #[command(subcommand)]
    command: TestCommands,
}

#[derive(clap::Subcommand)]
enum TestCommands {
    #[command(alias = "cook")]
    Chef {
        #[arg(short = 'a', long)]
        auto: bool,
    },
}

#[test]
fn chef_without_flags() {
    let cli = TestCli::try_parse_from(["sparrowdb-chef", "chef"]).unwrap();
    let TestCommands::Chef { auto } = cli.command;
    assert!(!auto);
}

#[test]
fn chef_with_auto_long() {
    let cli = TestCli::try_parse_from(["sparrowdb-chef", "chef", "--auto"]).unwrap();
    let TestCommands::Chef { auto } = cli.command;
    assert!(auto);
}

#[test]
fn chef_with_auto_short() {
    let cli = TestCli::try_parse_from(["sparrowdb-chef", "chef", "-a"]).unwrap();
    let TestCommands::Chef { auto } = cli.command;
    assert!(auto);
}

#[test]
fn cook_alias_works() {
    let cli = TestCli::try_parse_from(["sparrowdb-chef", "cook"]).unwrap();
    let TestCommands::Chef { auto } = cli.command;
    assert!(!auto);
}

#[test]
fn cook_alias_with_auto() {
    let cli = TestCli::try_parse_from(["sparrowdb-chef", "cook", "--auto"]).unwrap();
    let TestCommands::Chef { auto } = cli.command;
    assert!(auto);
}

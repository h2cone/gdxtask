use anyhow::Result;
use clap::{Parser, Subcommand};
use gdxtask::{addons, export, paths, run, update};

#[derive(Debug, Parser)]
#[command(name = "xtask", about = "Project workflow tasks")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Run(run::RunArgs),
    Export(export::ExportArgs),
    UpdateGdext(update::gdext::UpdateGdextArgs),
    UpdateGodotAddons(UpdateGodotAddonsCommand),
}

#[derive(Debug, clap::Args)]
struct UpdateGodotAddonsCommand {
    #[command(flatten)]
    args: addons::UpdateGodotAddonsArgs,
    #[arg(long, default_value = "all")]
    addon: String,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let paths = paths::ProjectPaths::discover(&paths::Layout::default())?;

    match cli.command {
        Command::Run(args) => run::execute(&paths, args),
        Command::Export(args) => export::execute(&paths, args),
        Command::UpdateGdext(args) => update::gdext::execute(&paths, args),
        Command::UpdateGodotAddons(command) => {
            let selection = if command.addon.eq_ignore_ascii_case("all") {
                addons::AddonSelection::All
            } else {
                addons::AddonSelection::One(&command.addon)
            };
            addons::execute(&paths, MY_ADDONS, selection, command.args)
        }
    }
}

const MY_ADDONS: &[addons::AddonSpec] = &[
    addons::AddonSpec {
        label: "LDtk Importer",
        repo: "heygleeson/godot-ldtk-importer",
        package_dir: "ldtk-importer",
        target_dir: "addons/ldtk-importer",
        download: addons::DownloadKind::ReleaseAssetZip,
    },
    addons::AddonSpec {
        label: "Aseprite Wizard",
        repo: "viniciusgerevini/godot-aseprite-wizard",
        package_dir: "AsepriteWizard",
        target_dir: "addons/AsepriteWizard",
        download: addons::DownloadKind::Zipball,
    },
];

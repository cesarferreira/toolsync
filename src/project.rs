use std::collections::HashSet;
use std::fs;
use std::path::Path;

use clap::ValueEnum;

#[derive(Clone, Copy, Debug, Default, ValueEnum)]
pub enum IconStyle {
    #[default]
    Emoji,
    Nerd,
    None,
}

impl IconStyle {
    pub fn label(self, name: &str, root: Option<&Path>) -> String {
        let project = || root.and_then(detect).unwrap_or(&REPOSITORY);
        match self {
            Self::Emoji => format!("{} {name}", project().emoji),
            Self::Nerd => format!("{} {name}", project().nerd),
            Self::None => name.to_owned(),
        }
    }
}

struct Project {
    markers: &'static [&'static str],
    emoji: &'static str,
    nerd: &'static str,
}

impl Project {
    const fn new(
        markers: &'static [&'static str],
        emoji: &'static str,
        nerd: &'static str,
    ) -> Self {
        Self {
            markers,
            emoji,
            nerd,
        }
    }
}

const REPOSITORY: Project = Project::new(&[], "📁", "\u{f401}");

// Look for specific project manifests before generic tooling and loose files.
// Use single-character emoji. Variation selectors can affect terminal column widths during redraws.
// Nerd Font code points come from the upstream glyphnames.json.
const PROJECTS: &[Project] = &[
    Project::new(&["Cargo.toml"], "🦀", "\u{e7a8}"),
    Project::new(&["go.mod", "go.work"], "🐹", "\u{e724}"),
    Project::new(
        &["AndroidManifest.xml", "app/src/main/AndroidManifest.xml"],
        "🤖",
        "\u{e70e}",
    ),
    Project::new(&["ProjectSettings/ProjectVersion.txt"], "🎮", "\u{e721}"),
    Project::new(&["project.godot"], "👾", "\u{e7ee}"),
    Project::new(
        &["Package.swift", "*.xcodeproj", "*.xcworkspace"],
        "🐦",
        "\u{e755}",
    ),
    Project::new(&["build.zig", "build.zig.zon"], "⚡", "\u{e8ef}"),
    Project::new(&["pubspec.yaml"], "🎯", "\u{e798}"),
    Project::new(&["mix.exs"], "💧", "\u{e7cd}"),
    Project::new(&["gleam.toml"], "✨", "\u{e914}"),
    Project::new(&["rebar.config"], "📞", "\u{e7b1}"),
    Project::new(&["build.sbt"], "🔺", "\u{e737}"),
    Project::new(&["deps.edn", "project.clj"], "🌿", "\u{e768}"),
    Project::new(&["cabal.project", "stack.yaml", "*.cabal"], "λ", "\u{e777}"),
    Project::new(&["dune-project", "*.opam"], "🐫", "\u{e84e}"),
    Project::new(&["shard.yml"], "🔮", "\u{e7ac}"),
    Project::new(&["*.nimble"], "👑", "\u{e841}"),
    Project::new(&["composer.json"], "🐘", "\u{e73d}"),
    Project::new(&["Gemfile", "*.gemspec"], "💎", "\u{e739}"),
    Project::new(&["pom.xml"], "☕", "\u{e738}"),
    Project::new(
        &[
            "*.csproj",
            "*.fsproj",
            "*.vbproj",
            "*.sln",
            "*.slnx",
            "Directory.Build.props",
        ],
        "🟪",
        "\u{e77f}",
    ),
    Project::new(&["deno.json", "deno.jsonc"], "🦕", "\u{e7c0}"),
    Project::new(
        &["tsconfig.json", "tsconfig.base.json", "tsconfig.app.json"],
        "🔷",
        "\u{e628}",
    ),
    Project::new(
        &[
            "pyproject.toml",
            "setup.py",
            "setup.cfg",
            "requirements.txt",
            "Pipfile",
        ],
        "🐍",
        "\u{e73c}",
    ),
    Project::new(&["package.json"], "🟨", "\u{e781}"),
    Project::new(
        &[
            "settings.gradle",
            "settings.gradle.kts",
            "build.gradle",
            "build.gradle.kts",
        ],
        "🧰",
        "\u{e7f2}",
    ),
    Project::new(&["*.kt"], "🟣", "\u{e81b}"),
    Project::new(&["*.rockspec", ".luarc.json"], "🌙", "\u{e826}"),
    Project::new(&["cpanfile", "Makefile.PL", "Build.PL"], "🐪", "\u{e769}"),
    Project::new(&["JuliaProject.toml", "Project.toml"], "🟢", "\u{e80d}"),
    Project::new(&["*.Rproj", "renv.lock"], "📊", "\u{e881}"),
    Project::new(
        &[
            "CMakeLists.txt",
            "meson.build",
            "configure.ac",
            "*.cpp",
            "*.cc",
            "*.cxx",
            "*.c",
        ],
        "🔩",
        "\u{e7a3}",
    ),
    Project::new(&["flake.nix", "default.nix", "shell.nix"], "🧊", "\u{e843}"),
    Project::new(&["*.tf", "*.tf.json"], "🧱", "\u{e8bd}"),
    Project::new(
        &[
            "Dockerfile",
            "Containerfile",
            "compose.yaml",
            "compose.yml",
            "docker-compose.yml",
            "docker-compose.yaml",
        ],
        "🐳",
        "\u{e7b0}",
    ),
    Project::new(&["index.html"], "🌐", "\u{e736}"),
    Project::new(
        &[
            "Brewfile",
            ".chezmoi.toml.tmpl",
            ".bashrc",
            ".zshrc",
            "*.sh",
            "*.zsh",
        ],
        "🔧",
        "\u{ea85}",
    ),
];

fn detect(root: &Path) -> Option<&'static Project> {
    let entries: HashSet<_> = fs::read_dir(root)
        .ok()?
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            let kind = entry.file_type().ok()?;
            let is_marker = kind.is_file()
                || (kind.is_symlink() && entry.path().is_file())
                || (kind.is_dir()
                    && (name.ends_with(".xcodeproj") || name.ends_with(".xcworkspace")));
            is_marker.then_some(name)
        })
        .collect();

    PROJECTS.iter().find(|project| {
        project.markers.iter().any(|marker| {
            if let Some(suffix) = marker.strip_prefix('*') {
                entries.iter().any(|name| name.ends_with(suffix))
            } else if marker.contains('/') {
                root.join(marker).is_file()
            } else {
                entries.contains(*marker)
            }
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_emoji_use_one_character() {
        for project in PROJECTS.iter().chain([&REPOSITORY]) {
            assert_eq!(project.emoji.chars().count(), 1, "{}", project.emoji);
        }
    }

    #[test]
    fn matches_files_suffixes_and_nested_manifests() {
        for (file, name, emoji) in [
            ("go.mod", "service", "🐹"),
            ("pyproject.toml", "script", "🐍"),
            ("package.json", "site", "🟨"),
            ("Backend.slnx", "backend", "🟪"),
            ("main.tf", "infra", "🧱"),
            ("Brewfile", "dotfiles", "🔧"),
            ("app/src/main/AndroidManifest.xml", "mobile", "🤖"),
        ] {
            let repo = tempfile::tempdir().unwrap();
            let marker = repo.path().join(file);
            fs::create_dir_all(marker.parent().unwrap()).unwrap();
            fs::write(marker, "").unwrap();
            assert_eq!(
                IconStyle::Emoji.label(name, Some(repo.path())),
                format!("{emoji} {name}")
            );
        }

        let repo = tempfile::tempdir().unwrap();
        fs::create_dir(repo.path().join("App.xcodeproj")).unwrap();
        assert_eq!(IconStyle::Emoji.label("app", Some(repo.path())), "🐦 app");
    }

    #[test]
    fn prefers_specific_markers_in_mixed_repos() {
        let repo = tempfile::tempdir().unwrap();
        for file in ["Cargo.toml", "tsconfig.json", "package.json", "Dockerfile"] {
            fs::write(repo.path().join(file), "").unwrap();
        }
        assert_eq!(IconStyle::Emoji.label("tool", Some(repo.path())), "🦀 tool");
        assert_eq!(
            IconStyle::Nerd.label("tool", Some(repo.path())),
            "\u{e7a8} tool"
        );
        assert_eq!(IconStyle::None.label("tool", Some(repo.path())), "tool");

        fs::remove_file(repo.path().join("Cargo.toml")).unwrap();
        assert_eq!(IconStyle::Emoji.label("tool", Some(repo.path())), "🔷 tool");
    }

    #[test]
    fn falls_back_for_unknown_or_unavailable_checkouts() {
        let repo = tempfile::tempdir().unwrap();
        fs::write(repo.path().join("README.md"), "").unwrap();
        assert_eq!(
            IconStyle::Emoji.label("notes", Some(repo.path())),
            "📁 notes"
        );
        assert_eq!(
            IconStyle::Nerd.label("notes", Some(&repo.path().join("missing"))),
            "\u{f401} notes"
        );
    }
}

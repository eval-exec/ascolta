{
  description = "ascolta — a modern, feature-rich terminal client for MPD";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" ];
      forAllSystems = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
    in {
      packages = forAllSystems (pkgs: {
        default = pkgs.rustPlatform.buildRustPackage {
          pname = "ascolta";
          version = "0.0.1";
          src = pkgs.lib.cleanSource ./.;
          cargoLock.lockFile = ./Cargo.lock;

          nativeBuildInputs = [ pkgs.installShellFiles pkgs.pkg-config ];
          buildInputs = [ pkgs.sqlite ];

          postInstall = ''
            installManPage docs/ascolta.1
            installShellCompletion --cmd ascolta \
              --bash <($out/bin/ascolta completions bash) \
              --zsh <($out/bin/ascolta completions zsh) \
              --fish <($out/bin/ascolta completions fish)
            install -Dm644 config.example.toml \
              "$out/share/doc/ascolta/config.example.toml"
            install -Dm644 README.md "$out/share/doc/ascolta/README.md"
          '';

          meta = with pkgs.lib; {
            description = "A modern, feature-rich terminal client for MPD";
            homepage = "https://github.com/eval-exec/ascolta";
            license = licenses.mit;
            mainProgram = "ascolta";
            platforms = platforms.linux;
          };
        };
      });

      devShells = forAllSystems (pkgs: {
        default = pkgs.mkShell {
          packages = with pkgs; [
            cargo
            rustc
            clippy
            rustfmt
            pkg-config
            just
            cargo-nextest
            mpd
            mpc
            ffmpeg
            playerctl
          ];
        };
      });

      formatter = forAllSystems (pkgs: pkgs.nixfmt-rfc-style);
    };
}

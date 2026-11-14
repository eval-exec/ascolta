{
  description = "neo-ncmp — a modern, feature-rich terminal client for MPD";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" ];
      forAllSystems = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
    in {
      packages = forAllSystems (pkgs: {
        default = pkgs.rustPlatform.buildRustPackage {
          pname = "neo-ncmp";
          version = "0.1.0";
          src = pkgs.lib.cleanSource ./.;
          cargoLock.lockFile = ./Cargo.lock;

          nativeBuildInputs = [ pkgs.installShellFiles ];

          postInstall = ''
            installManPage docs/neo-ncmp.1
            install -Dm644 config.example.toml \
              "$out/share/doc/neo-ncmp/config.example.toml"
            install -Dm644 README.md "$out/share/doc/neo-ncmp/README.md"
          '';

          meta = with pkgs.lib; {
            description = "A modern, feature-rich terminal client for MPD";
            homepage = "https://github.com/eval-exec/neo-ncmp";
            license = licenses.mit;
            mainProgram = "neo-ncmp";
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

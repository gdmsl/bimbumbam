{
  description = "bimbumbam — a toddler-friendly keyboard basher with colorful visual effects";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};

        manifest = (pkgs.lib.importTOML ./Cargo.toml).package;

        runtimeDeps = with pkgs; [
          wayland
          libxkbcommon
          fontconfig
          vulkan-loader
          alsa-lib
        ];

        nativeBuildInputs = with pkgs; [
          pkg-config
        ];

        rpath = pkgs.lib.makeLibraryPath runtimeDeps;

        bimbumbam = pkgs.rustPlatform.buildRustPackage {
          pname = manifest.name;
          version = manifest.version;

          src = pkgs.lib.cleanSourceWith {
            src = ./.;
            filter = path: type:
              let base = baseNameOf (toString path); in
              base != "target" && base != ".direnv";
          };

          cargoLock.lockFile = ./Cargo.lock;

          inherit nativeBuildInputs;
          buildInputs = runtimeDeps;

          postFixup = ''
            patchelf --set-rpath "${rpath}" $out/bin/${manifest.name}
          '';

          meta = with pkgs.lib; {
            inherit (manifest) description;
            mainProgram = manifest.name;
            platforms = platforms.linux;
          };
        };
      in {
        packages.default = bimbumbam;
        packages.bimbumbam = bimbumbam;

        apps.default = {
          type = "app";
          program = "${bimbumbam}/bin/${manifest.name}";
          meta = bimbumbam.meta;
        };

        devShells.default = pkgs.mkShell {
          inherit nativeBuildInputs;

          buildInputs = runtimeDeps ++ (with pkgs; [
            cargo
            rustc
            rustfmt
            clippy
            rust-analyzer
          ]);

          LD_LIBRARY_PATH = builtins.concatStringsSep ":" [
            rpath
            "/run/opengl-driver/lib"
          ];

          shellHook = ''
            export XDG_DATA_DIRS="/run/opengl-driver/share''${XDG_DATA_DIRS:+:$XDG_DATA_DIRS}"
          '';
        };

        formatter = pkgs.nixpkgs-fmt;
      });
}

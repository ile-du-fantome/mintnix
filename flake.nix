{
  description = "mintnix -- Linux-only fork of drg-mint-notag, built on stable Rust";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };

    # Native Linux Oodle libraries for mods distributed with Oodle-compressed
    # paks
    oodleue-src = {
      url = "github:WorkingRobot/OodleUE";
      flake = false;
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      rust-overlay,
      oodleue-src,
      ...
    }:
    let
      system = "x86_64-linux";
      pkgs = import nixpkgs {
        inherit system;
        overlays = [ rust-overlay.overlays.default ];
      };
      inherit (pkgs) lib;

      manifest = lib.importTOML ./Cargo.toml;
      packageName = manifest.package.name;
      packageVersion = manifest.workspace.package.version;
      revision = self.shortRev or self.dirtyShortRev or "dirty";
      version = "${packageVersion}-unstable-${revision}";

      rustToolchain = pkgs.rust-bin.stable.latest.default.override {
        targets = [
          "x86_64-unknown-linux-gnu"
          "x86_64-pc-windows-gnu"
        ];
      };
      rustPlatform = pkgs.makeRustPlatform {
        cargo = rustToolchain;
        rustc = rustToolchain;
      };

      # mintnix cross-compiles the hook cdylib for the Windows game running
      # under Proton. Stable cargo cannot use artifact dependencies, so the
      # hook is built separately and its path is passed to the main build
      mingwPkgs = pkgs.pkgsCross.mingwW64;
      mingwCompiler = mingwPkgs.buildPackages.gcc;
      mingwTool =
        name: "${mingwCompiler}/bin/${mingwCompiler.targetPrefix}${name}";

      libs = with pkgs; [
        gtk3
        libGL
        atk
        libxkbcommon
        wayland
        stdenv.cc.cc.lib
      ];
      libraryPath = lib.makeLibraryPath libs;

      package = rustPlatform.buildRustPackage {
        pname = packageName;
        inherit version;

        src = self;

        cargoLock = {
          lockFile = ./Cargo.lock;
          allowBuiltinFetchGit = true;
        };

        nativeBuildInputs = [
          pkgs.pkg-config
          pkgs.makeWrapper
          pkgs.clang
          pkgs.mold
          mingwCompiler
        ];
        buildInputs = libs;

        env = {
          CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUSTFLAGS = "-L ${mingwPkgs.windows.pthreads}/lib";
          CC_x86_64_pc_windows_gnu = mingwTool "cc";
          AR_x86_64_pc_windows_gnu = mingwTool "ar";
        };

        preConfigure = ''
          export LD_LIBRARY_PATH="${libraryPath}"
        '';

        preBuild = ''
          cargo build --release --offline -p hook --target x86_64-pc-windows-gnu
          export MINT_HOOK_DLL="$PWD/target/x86_64-pc-windows-gnu/release/hook.dll"
        '';

        cargoBuildFlags = [
          "--package"
          packageName
        ];

        auditable = false;
        doCheck = false;

        postInstall = ''
          oodle_sdk="${oodleue-src}/Engine/Source/Programs/Shared/EpicGames.Oodle/Sdk"
          oodle_lib="$(find "$oodle_sdk" -path '*/linux/lib/liboo2corelinux64.so.9' -print -quit)"
          if [ -z "$oodle_lib" ]; then
            echo "mintnix: liboo2corelinux64.so.9 not found under $oodle_sdk" 1>&2
            echo "The OodleUE input no longer provides the expected Linux library." 1>&2
            exit 1
          fi
          install -m 0444 "$oodle_lib" "$out/bin/liboo2corelinux64.so.9"

          wrapProgram $out/bin/${packageName} \
            --prefix LD_LIBRARY_PATH : "${libraryPath}" \
            --prefix XDG_DATA_DIRS : "${pkgs.gtk3}/share/gsettings-schemas/${pkgs.gtk3.name}"
        '';

        meta = {
          description = "Deep Rock Galactic mod integration tool (mintnix)";
          homepage = "https://github.com/ile-du-fantome/mintnix";
          changelog = "https://github.com/ile-du-fantome/mintnix/blob/master/CHANGELOG.md";
          license = lib.licenses.mit;
          mainProgram = packageName;
          platforms = [ "x86_64-linux" ];
        };
      };

      devShell = pkgs.mkShell {
        name = "mintnix";

        buildInputs = [
          rustToolchain
          pkgs.pkg-config
          mingwCompiler
          pkgs.clang
          pkgs.mold
          pkgs.gcc
          pkgs.git
          pkgs.just
          pkgs.rust-analyzer
        ] ++ libs;

        LD_LIBRARY_PATH = libraryPath;
        CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUSTFLAGS = "-L ${mingwPkgs.windows.pthreads}/lib";

        # Needed by cc-rs when building C code for the hook target.
        CC_x86_64_pc_windows_gnu = mingwTool "cc";
        AR_x86_64_pc_windows_gnu = mingwTool "ar";
      };
    in
    {
      packages.${system} = {
        ${packageName} = package;
        default = package;
      };

      devShells.${system}.default = devShell;
    };
}

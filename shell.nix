{ pkgs ? import <nixpkgs> {} }:

pkgs.mkShell {
  nativeBuildInputs = with pkgs; [
    pkg-config
    cargo
    rustc
    clang
  ];

  buildInputs = with pkgs; [
    opencv
    llvmPackages.llvm      # Provides llvm-config
    llvmPackages.libclang  # Provides libclang.so
  ];

  LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";
  LLVM_CONFIG_PATH = "${pkgs.llvmPackages.llvm.dev}/bin/llvm-config";
  BINDGEN_EXTRA_CLANG_ARGS = ''
    -isystem ${pkgs.llvmPackages.libclang.lib}/lib/clang/${pkgs.lib.getVersion pkgs.clang}/include
    -isystem ${pkgs.glibc.dev}/include
  '';
}
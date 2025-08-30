{ pkgs, lib, config, inputs, ... }:

{
  languages.python.enable = true;
  languages.rust.enable = true;

  packages = [
    pkgs.clang
    pkgs.cudaPackages.cudnn
    pkgs.cudaPackages.cuda_cudart

    pkgs.python313Packages.torch
    pkgs.python313Packages.numpy
    pkgs.python313Packages.scipy
    pkgs.python313Packages.pyqt5
    pkgs.python313Packages.pyqtgraph
    pkgs.python313Packages.darkdetect
  ];

  env.PYTHONPATH = "${./.}/python";
  env.CUDA_PATH = "${pkgs.cudatoolkit}";
  env.LIBCLANG_PATH = "${pkgs.libclang.lib}/lib";
  env.LD_LIBRARY_PATH = lib.makeLibraryPath [
    pkgs.cudatoolkit.lib
    pkgs.cudaPackages.cudnn
    pkgs.cudaPackages.cuda_cudart
    pkgs.libclang.lib
  ];
}

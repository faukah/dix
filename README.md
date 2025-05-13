# Diff Nix

A blazingly fast tool to diff Nix related things.

Currently only supports closures (a derivation graph, such as a system build or
package).

![output of `dix /nix/var/nix/profiles/system-69-link/ /run/current-system`](.github/dix.png)

## Usage

```bash
$ dix --help

Diff Nix

Usage: dix [OPTIONS] <OLD_PATH> <NEW_PATH>

Arguments:
  <OLD_PATH>  
  <NEW_PATH>  

Options:
  -v, --verbose...  Increase logging verbosity
  -q, --quiet...    Decrease logging verbosity
  -h, --help        Print help
  -V, --version     Print version

$ dix /nix/var/profiles/system-69-link /run/current-system
```

## Contributing

If you have any problems, feature requests or want to contribute code or want to
provide input in some other way, feel free to create an issue or a pull request!

## Thanks

Huge thanks to [nvd](https://git.sr.ht/~khumba/nvd) for the original idea! Dix
is heavily inspired by this and basically just a "Rewrite it in Rust" version of
nvd, with a few things like version diffing done better.

Furthermore, many thanks to the amazing people who made this projects possible
by contributing code and offering advice:

- [@RGBCube](https://github.com/RGBCube) - Giving the codebase a deep scrub.
- [@Dragyx](https://github.com/Dragyx) - Cool SQL queries. Much of dix's speed
  is thanks to him.
- [@NotAShelf](https://github.com/NotAShelf) - Implementing proper error
  handling.

## License

Dix is licensed under [GPLv3](LICENSE.md). See the license file for more
details.

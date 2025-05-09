# Diff Nix

A tool to diff any Nix related thing.

Currently only supports closures (a derivation graph, such as a system build or
package).

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

## Output

![output of `dix /nix/var/nix/profiles/system-165-link/ /run/current-system`](.github/dix.png)

## License

Dix: Diff Nix Copyright (C) 2025-present bloxx12

This program is free software: you can redistribute it and/or modify it under
the terms of the GNU General Public License as published by the Free Software
Foundation, either version 3 of the License, or (at your option) any later
version.

This program is distributed in the hope that it will be useful, but WITHOUT ANY
WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS FOR A
PARTICULAR PURPOSE. See the GNU General Public License for more details.

You should have received a copy of the GNU General Public License along with
this program. If not, see <https://www.gnu.org/licenses/>.

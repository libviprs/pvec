# Migrating within and beyond v1

This is what a v1.x reader or writer can rely on. It restates the rules of [`COMPATIBILITY.md`](COMPATIBILITY.md) and the
container layout of [`CONTAINER.md`](CONTAINER.md) for someone who has to decide whether an upgrade is safe, and adds no rule of its own.

## What a v1.x reader must keep working

* Every file whose header says container 1.0 and whose manifest asks for no capability the reader lacks. The header layout,
  the byte-order marker, the block directory entry and the integrity tables read as they do today.
* Every capability in the v1 registry snapshot (`docs/capabilities.toml`) at the version and minimum reader it states.
* Skipping an unknown optional block without parsing its payload, and refusing an unknown must-understand block by name.
* The numeric policy and the security limits as defaults: a reader may be configured stricter, never silently looser.
* The packed WASM layouts at `wasm_abi_version` 1.

## What may change within v1.x

* New optional blocks and new capabilities added through the extension rules, which an older reader skips or refuses by name.
* A new minor container version that only adds optional fields after the existing header.
* Writer defaults that do not change what a reader must accept (codec choice, block packing, index tuning).
* Tightened limits in the browser preset, and faster or smaller readers behind the same behaviour.

## What requires v2

* Any change to the meaning, size or position of an existing header or directory field.
* Removing a capability, or raising its minimum reader beyond what the compatibility rules allow within a major version.
* Loosening a P1.2 tolerance or a P4.8 limit as a default, or changing the semantic profile.
* A new `wasm_abi_version`, or any change to the packed layouts of version 1.

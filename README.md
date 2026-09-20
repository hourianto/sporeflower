# Sporeflower

Sporeflower is a Java decompiler tailored for J2ME-era CLDC/MIDP `.jar` files. It is a fork of [Vineflower](https://github.com/Vineflower/vineflower).

It also includes the `j2me` command-line toolkit for renaming, semantic mappings, and recompilation.

Sporeflower improves on Vineflower's output for some J2ME targets, but it's not considered stable at the moment.

> [!WARNING]
> AI usage disclosure: Sporeflower is exclusively developed by agentic LLMs, so please be mindful of [Vineflower's AI policy](https://github.com/Vineflower/vineflower/blob/master/CONTRIBUTING.md#ai-policy) if you plan to port changes upstream.

## Installation

See [INSTALL.md](docs/INSTALL.md) for installation and getting started.

## Special Thanks

Sporeflower would not exist without [Vineflower](https://github.com/Vineflower/vineflower), its maintainers, and its contributors. This fork also inherits from the broader Fernflower/Vineflower lineage, so special thanks to:

* [Stiver](https://blog.jetbrains.com/idea/2024/11/in-memory-of-stiver/), for creating Fernflower
* JetBrains, for maintaining Fernflower
* MinecraftForge Team, for maintaining ForgeFlower
* FabricMC Team, for maintaining Fabric's fork of Fernflower
* Vineflower maintainers and contributors, for the upstream project this fork is based on
* CFR, for its large suite of very useful tests

## Upstream reports

Work on Sporeflower led to the following bug reports in Vineflower. Some may already be fixed upstream; see the linked issues for their current status.

* [Downcasts affecting exceptions and field selection](https://github.com/Vineflower/vineflower/issues/623)
* [Array casts after `Object` parameter reassignment](https://github.com/Vineflower/vineflower/issues/622)
* [Typed receivers for null-initialized locals](https://github.com/Vineflower/vineflower/issues/621)
* [`@Override` for inaccessible superclass methods](https://github.com/Vineflower/vineflower/issues/620)
* [Switch break label scope](https://github.com/Vineflower/vineflower/issues/619)
* [Primitive array type joins](https://github.com/Vineflower/vineflower/issues/618)
* [Casts in unrelated reference comparisons](https://github.com/Vineflower/vineflower/issues/617)
* [Single evaluation of duplicated field values](https://github.com/Vineflower/vineflower/issues/607)
* [Object/array overload selection](https://github.com/Vineflower/vineflower/issues/582)
* [Receiver casts for private base methods](https://github.com/Vineflower/vineflower/issues/581)
* [Preserving integer local ranges](https://github.com/Vineflower/vineflower/issues/579)
* [Byte switch variable reused as an int](https://github.com/Vineflower/vineflower/issues/578)
* [Invalid internal type for `byte[]` locals](https://github.com/Vineflower/vineflower/issues/577)
* [Float ternary assignments](https://github.com/Vineflower/vineflower/issues/576)
* [Standalone unboxing calls](https://github.com/Vineflower/vineflower/issues/575)
* [Reference/int local-slot reuse](https://github.com/Vineflower/vineflower/issues/574)
* [String concatenation type inference](https://github.com/Vineflower/vineflower/issues/573)
* [Static initialization order](https://github.com/Vineflower/vineflower/issues/572)
* [Decrement moved into a short-circuit condition](https://github.com/Vineflower/vineflower/issues/569)

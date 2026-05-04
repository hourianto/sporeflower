# Sporeflower

Sporeflower is a Java decompiler tailored for J2ME-era CLDC/MIDP `.jar` files. It is a fork of [Vineflower](https://github.com/Vineflower/vineflower).

Sporeflower improves on Vineflower's output for some J2ME targets, but it's not considered stable at the moment.

> [!WARNING]
> AI usage disclosure: Sporeflower is exclusively developed by agentic LLMs, so please be mindful of [Vineflower's AI policy](https://github.com/Vineflower/vineflower/blob/master/CONTRIBUTING.md#ai-policy) if you plan to port changes upstream.

## Building

Sporeflower can be built with `./gradlew build`.

## Special Thanks

Sporeflower would not exist without [Vineflower](https://github.com/Vineflower/vineflower), its maintainers, and its contributors. This fork also inherits from the broader Fernflower/Vineflower lineage, so special thanks to:

* [Stiver](https://blog.jetbrains.com/idea/2024/11/in-memory-of-stiver/), for creating Fernflower
* JetBrains, for maintaining Fernflower
* MinecraftForge Team, for maintaining ForgeFlower
* FabricMC Team, for maintaining Fabric's fork of Fernflower
* Vineflower maintainers and contributors, for the upstream project this fork is based on
* CFR, for its large suite of very useful tests

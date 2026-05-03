# Vineflower J2ME

This fork is a legacy Java and J2ME-oriented decompiler focused on old jars, MIDlets, and obfuscated pre-modern bytecode.

The fork keeps the low-level machinery that helps with old class files:
- Java 1.1-8 bytecode handling, including JSR/RET, StackMap-era classes, inner-class recovery, synthetic access repair, and enum/switch cleanup
- JASM and regression coverage for malformed or obfuscated legacy bytecode
- Variable renaming support for hard-to-read jars

Modern JVM-language and Java 9+ surface resugaring has intentionally been removed.

## Use

Vineflower can be used from the console or as a library. To run Vineflower from the command line, download the latest release from the [Releases tab](https://github.com/Vineflower/vineflower/releases).
You can then run Vineflower with `java -jar vineflower.jar <arguments> <source> <destination>`.
`<arguments>` is the list of [commandline arguments](https://vineflower.org/usage/) that you want to pass to the decompiler.
`<source>` can be a jar, zip, folder, or class file, and `<destination>` can be a folder, zip, jar, or omitted to print to the console.

This fork currently builds and runs on a modern JDK, but its decompilation target is legacy Java/J2ME bytecode.
Vineflower can be addded as a dependency in gradle with:
```groovy
dependencies {
    implementation 'org.vineflower:vineflower:<version>'
}
```

### Building
Vineflower can be built simply with `./gradlew build`.

### Support
This is a focused fork; upstream Vineflower documentation may describe features that are no longer present here.

## Contributing
Contributions are always welcome! [The website](https://vineflower.org/development/) has detailed instructions on how to set up Vineflower development, as well as information on debugging.
When submitting pull requests, please target the latest `develop/1.xx.y` branch.

### Special Thanks
Vineflower is a fork of Jetbrains' Fernflower, MinecraftForge's ForgeFlower, FabricMC's fork of Fernflower, and a direct continuation of work on Quiltflower. Special thanks to:

* [Stiver](https://blog.jetbrains.com/idea/2024/11/in-memory-of-stiver/), for creating Fernflower
* JetBrains, for maintaining Fernflower
* MinecraftForge Team, for maintaining ForgeFlower
* FabricMC Team, for maintaining Fabric's fork of Fernflower
* CFR, for its large suite of very useful tests

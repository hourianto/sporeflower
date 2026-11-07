# API catalog provenance

The catalogs contain declarations extracted from SDK JARs or transcribed from
Javadoc and API sources. The initial extraction used the toolkit from the
[legacy branch](https://github.com/hourianto/sporeflower/tree/legacy).
See [API declarations](README.md) for the format and use.

* **[MicroEmulator 2.0.4](https://github.com/barteo/microemu/tree/microemulator_2_0_4):**
  CLDC 1.0/1.1, MIDP 2.0, JSR 75/82/120/135, Nokia UI and Siemens declarations
  from the corresponding `org.microemu` Maven artifacts.
* **[KEmulator 2.21.4](https://github.com/shinovon/KEmulator/releases/tag/v2.21.4):**
  `mascotv3-2.21.4`, `jsr256-sensor`, `jsr211-sdk`, `jsr177-crypto`,
  `pigler-api`, `nokiaui-sdk` and `samsung-api`, from its UEI API JARs.
* **[Archived Samsung and Nokia Javadoc](https://github.com/nikita36078/J2ME_Docs/tree/fbfdfed7009369b6bf7a5cce39b8a13f178c2c69/docs):**
  `samsung-audioclip` from Samsung API 1.0 (July 2003);
  `nokia-services-api` from `nokiaapi2`;
  `nokia-payment-api` from Nokia In-App Purchase API 1.0.
* **[Nokia S40 SDK](https://github.com/0x726564/S40-SDK-Classes/tree/decec09b9479214ce790ed3ad2e9e07ef084bdc6):**
  `nokia-s40-internal-api` from `S40v6/jar/cldc11.jar`.
* **[Siemens extensions in KEmulator](https://github.com/shinovon/KEmulator/tree/4dfdfe31243c9bdbcffa987101e27421618e79e8/src/oem/com/siemens/mp):**
  `siemens-extended-api` adds File and VolumeControl declarations.
* **KEmulator source declarations:** `kemulator-extensions` contains
  [TextEditorUtils](https://github.com/shinovon/KEmulator/blob/4dfdfe31243c9bdbcffa987101e27421618e79e8/src/nokia/com/nokia/mid/ui/s40/TextEditorUtils.java),
  [Samsung LCDLight](https://github.com/shinovon/KEmulator/blob/4dfdfe31243c9bdbcffa987101e27421618e79e8/src/oem/com/samsung/util/LCDLight.java),
  [Sprint Location](https://github.com/shinovon/KEmulator/blob/4dfdfe31243c9bdbcffa987101e27421618e79e8/src/oem/com/sprintpcs/util/Location.java) and
  [MMPP BackLight](https://github.com/shinovon/KEmulator/blob/4dfdfe31243c9bdbcffa987101e27421618e79e8/src/oem/mmpp/media/BackLight.java).
* **Earlier SDK imports:** `blackberry-api-7.1.0`, `jsr179-location`, `jsr180-sip`,
  `jsr184`, `jsr211-chapi`, `jsr226-svg`, `jsr229-payment`, `jsr234-amms`,
  `jsr238-global`, `jsr75-pim`, `motorola-api`, `motorola-io-file`,
  `siemens-extra-api`, `siemens-jsr75-file` and `vodafone-api` came from the
  correspondingly named API JARs. Their original download locations were not recorded.

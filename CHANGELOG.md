# Changelog

## [0.4.0](https://github.com/bjornharvold/curlybrace/compare/v0.3.0...v0.4.0) (2026-10-16)


### Features

* **connections:** say what is wrong with a connection string, accept unicode hosts in from-uri ([#101](https://github.com/bjornharvold/curlybrace/issues/101)) ([6b41170](https://github.com/bjornharvold/curlybrace/commit/6b41170430a46b12f92226fbfe6ca2f626f1f5b3))


### Bug Fixes

* **connections:** accessibility and list polish for the connection manager ([#100](https://github.com/bjornharvold/curlybrace/issues/100)) ([e94a480](https://github.com/bjornharvold/curlybrace/commit/e94a48099fb291097171bfde51c733328380ffb6))
* **connections:** label list dates as utc, translate list separators, show only new banners in the edit dialog ([#102](https://github.com/bjornharvold/curlybrace/issues/102)) ([73a3afa](https://github.com/bjornharvold/curlybrace/commit/73a3afa69ec581435e3a69a64e243ae7b45c861c))
* **connections:** repair only invalid saved entries, end a test when the runtime closes, faster import ([#99](https://github.com/bjornharvold/curlybrace/issues/99)) ([fc7fb5d](https://github.com/bjornharvold/curlybrace/commit/fc7fb5d40efb7945c8cfd1efe5a13ea6c8cb3281))
* **connections:** stricter uri check, host-key questions that end with their job ([#96](https://github.com/bjornharvold/curlybrace/issues/96)) ([cc3e8ee](https://github.com/bjornharvold/curlybrace/commit/cc3e8ee72bf92e377f304bd8f6799d26b997e3bd))
* **ui:** mask the pasted connection string and wipe egui's editor state for it ([#98](https://github.com/bjornharvold/curlybrace/issues/98)) ([ed81752](https://github.com/bjornharvold/curlybrace/commit/ed81752a6173bccd1599f44ed94c5537cc92cda4))

## [0.3.0](https://github.com/bjornharvold/curlybrace/compare/v0.2.0...v0.3.0) (2026-10-15)


### Features

* **connections:** add a Test Connection runtime command that opens no session ([7f3e22a](https://github.com/bjornharvold/curlybrace/commit/7f3e22ab3dd7d22f022404936235bafa307ea039))
* **connections:** add all-or-nothing secret copy and a redacting SecretString ([87ec0ae](https://github.com/bjornharvold/curlybrace/commit/87ec0ae47038179bf24a5a169ecac4d3b5f616d9))
* **connections:** add folder and timestamp fields to saved connections ([cf04957](https://github.com/bjornharvold/curlybrace/commit/cf04957505fa88514584ba36614d437ae0e280f3))
* **connections:** add pure folder helpers ([4f936a4](https://github.com/bjornharvold/curlybrace/commit/4f936a459f7a09973aba5c517d182500b57111be))
* **connections:** add the edit dialog with folder, from/to uri and test connection ([3a8f165](https://github.com/bjornharvold/curlybrace/commit/3a8f165f1e2de82c7418978382df4978a2a51fac))
* **connections:** add the list view-model for filter, sort and folder grouping ([4f4ed47](https://github.com/bjornharvold/curlybrace/commit/4f4ed47e62b5a81c35329b3fe7986be628798e87))
* **connections:** add the manager list with filter, sortable columns, folders and a folder prompt ([6e570f9](https://github.com/bjornharvold/curlybrace/commit/6e570f949f1c6dc9ee67c4b2554c9a52f463be84))
* **connections:** clearer multi-connection UI and safer host-key prompts ([#90](https://github.com/bjornharvold/curlybrace/issues/90)) ([ca7270d](https://github.com/bjornharvold/curlybrace/commit/ca7270dc3e6efcde8a21b31408a1553e3e517fca))
* **connections:** close the dialog on a successful save and tidy folder and duplicate handling ([62c7531](https://github.com/bjornharvold/curlybrace/commit/62c75313a5df2be557e478debf3b434af61a62cb))
* **connections:** connection manager slice 1 (store, manager list, edit dialog) ([d0fe792](https://github.com/bjornharvold/curlybrace/commit/d0fe79205aa6809b76ab41b750034a00841d7884))
* **connections:** duplicate, folder operations and last-connected stamp ([5e8a72f](https://github.com/bjornharvold/curlybrace/commit/5e8a72f9f73d682093125fa508363a443a279afa))
* **connections:** import and export saved connections without secrets ([d966d7c](https://github.com/bjornharvold/curlybrace/commit/d966d7c697896acb68760892f7761267c8ccf391))
* **connections:** parse and build connection strings with a masked password ([8f970f8](https://github.com/bjornharvold/curlybrace/commit/8f970f8908efd1ed8054e77f63d5385c6e7bf1c1))
* **connections:** several simultaneous connections with a tree root per connection ([#88](https://github.com/bjornharvold/curlybrace/issues/88)) ([293a76b](https://github.com/bjornharvold/curlybrace/commit/293a76b205dc546d6f4c80ea3485468b8e49d011))
* **connections:** wire Test Connection and From/To URI into the UI state ([3aa5f89](https://github.com/bjornharvold/curlybrace/commit/3aa5f8915f3333191359bf8eb3eda2ea3d34c02a))
* **core:** classify a tls handshake failure apart from an unreachable server ([#92](https://github.com/bjornharvold/curlybrace/issues/92)) ([aa11d95](https://github.com/bjornharvold/curlybrace/commit/aa11d953d9cd2c6a42db080cd03de48dde5ac8e9))


### Bug Fixes

* **connections:** add duplicate_secrets failure text and prove duplicate rollback ([faf5d24](https://github.com/bjornharvold/curlybrace/commit/faf5d2463bd9ee0980895be412f191ee1d39c0df))
* **connections:** cap unique_name at the max length and reject invisible format characters in labels ([2530a9b](https://github.com/bjornharvold/curlybrace/commit/2530a9ba1eba31bd72b42c95d90d423251fb81e4))
* **connections:** cap unique_name at the name limit so duplicates always validate ([134c290](https://github.com/bjornharvold/curlybrace/commit/134c29021bd3eed669519783d1daa707b2606ea1))
* **connections:** classify save outcomes, show failures in the dialog and drop dialog state on connect ([dd3146e](https://github.com/bjornharvold/curlybrace/commit/dd3146eead5be2847a1b3e8dfdf129aa742bb895))
* **connections:** give import and export failures their own banners ([8aceedc](https://github.com/bjornharvold/curlybrace/commit/8aceedcb424c0442f3c486d83f41d6bf85094c1c))
* **connections:** give name and folder errors their own variants and keep the folder prompt open on rejection ([c64b2b7](https://github.com/bjornharvold/curlybrace/commit/c64b2b73cd2ba0b22fa04af474dd37ce7b2bd14d))
* **connections:** guard the redacting debug impl and show the whole test result ([c424491](https://github.com/bjornharvold/curlybrace/commit/c4244910f57ddbf0e1cf7d8ed6d4e4a3e72af1b2))
* **connections:** honour cleared secrets in test and build, reject the legacy key-password alias and more invisible characters ([dc39be6](https://github.com/bjornharvold/curlybrace/commit/dc39be63a1522ff15a4b5ad90e260e3fdfc5387d))
* **connections:** keep the edit dialog on outside clicks and tighten test, save and debug edge cases ([3642951](https://github.com/bjornharvold/curlybrace/commit/364295184458cccc2a93187f5d92bcf82f5bb35a))
* **connections:** keep the pasted connection string out of debug output and audit secrets ([2b01ac1](https://github.com/bjornharvold/curlybrace/commit/2b01ac1de4e37d16a66080a6657c0fb6a79685f2))
* **connections:** name the authentication combo and use sort markers that render ([1606b79](https://github.com/bjornharvold/curlybrace/commit/1606b79f1a028a8dc988e2c5eb42ba96c816727d))
* **connections:** repair labels saved by older releases at load instead of dropping them ([526e007](https://github.com/bjornharvold/curlybrace/commit/526e00751d23c46c2b25ef9dc4f20d609a9dd34e))
* **connections:** validate connection names and withhold the import notice when saving fails ([912e5d4](https://github.com/bjornharvold/curlybrace/commit/912e5d4f0d3b2333b81a06dbcf32f83ce0cd79cc))
* **connections:** validate hosts and reject malformed userinfo so password fragments never reach the stored uri ([b0a67a1](https://github.com/bjornharvold/curlybrace/commit/b0a67a1a3bc3f33da8e75cecdb085ad788d241c3))
* **ui:** local e2e flows and a lost server marks its root failed ([#91](https://github.com/bjornharvold/curlybrace/issues/91)) ([bead006](https://github.com/bjornharvold/curlybrace/commit/bead006ecac279b4328871d1fee19c86f1ef14e5))

## [0.2.0](https://github.com/bjornharvold/curlybrace/compare/v0.1.0...v0.2.0) (2026-10-14)


### Features

* **compare:** apply engine in core (slice 2a) ([#36](https://github.com/bjornharvold/curlybrace/issues/36)) ([09104c7](https://github.com/bjornharvold/curlybrace/commit/09104c77acf04f366f4f51ffca502b2e14bc7ad5))
* **compare:** apply UI (slice 2b) ([#39](https://github.com/bjornharvold/curlybrace/issues/39)) ([4d53b26](https://github.com/bjornharvold/curlybrace/commit/4d53b2619c12ba220e6f20289807766fe56a448d))
* **compare:** data compare slice 1 (engine, session protocol, read-only Compare workspace) ([#29](https://github.com/bjornharvold/curlybrace/issues/29)) ([d086ba4](https://github.com/bjornharvold/curlybrace/commit/d086ba44f6f591d1ef4f80e33b06f17768fb37aa))
* **compare:** export plans and reports (slice 3) ([#43](https://github.com/bjornharvold/curlybrace/issues/43)) ([c81496a](https://github.com/bjornharvold/curlybrace/commit/c81496aa7c365f5ef9d72301fc00e2a84dcfb571))
* **compare:** field-level apply (slice 2c) ([#41](https://github.com/bjornharvold/curlybrace/issues/41)) ([f34f90c](https://github.com/bjornharvold/curlybrace/commit/f34f90c8beb16b10b64b9f529ed6385e8377eae8))
* **compare:** find duplicate keys (slice 1b) ([#32](https://github.com/bjornharvold/curlybrace/issues/32)) ([f529df2](https://github.com/bjornharvold/curlybrace/commit/f529df2483f55567896bbda02fc79592ec4f62d7))
* **compare:** multi-pair compare UI (slice 1c) ([#34](https://github.com/bjornharvold/curlybrace/issues/34)) ([8b7a123](https://github.com/bjornharvold/curlybrace/commit/8b7a1239fddee01d07c93f0d9c50d8812d1646ec))

## 0.1.0 (2026-10-12)


### Features

* **app:** add the app icon ([#23](https://github.com/bjornharvold/curlybrace/issues/23)) ([1975e9e](https://github.com/bjornharvold/curlybrace/commit/1975e9e10ed1246951cd8ba4ea91bf68b9745374))
* **app:** wire runtime, state reducer and eframe shell ([fd348f7](https://github.com/bjornharvold/curlybrace/commit/fd348f734af41260ff185e7332cf3120cec87a42))
* **core:** add connection config model and validation ([ec4cf40](https://github.com/bjornharvold/curlybrace/commit/ec4cf40d40a438c23a835389462c7da550072ff1))
* **core:** add document crud service with id handling ([4e2635c](https://github.com/bjornharvold/curlybrace/commit/4e2635ccd71ed73346bfa989f62409dec67158a2))
* **core:** add document store trait, paging types and test fixtures ([a909ff8](https://github.com/bjornharvold/curlybrace/commit/a909ff8dc56494840f35fd279b8ef13bf8892131))
* **core:** add mongo store and async runtime bridge ([7e5f2cf](https://github.com/bjornharvold/curlybrace/commit/7e5f2cf5fd1cbd2fd746e3e44cd0766dba3f4734))
* **core:** add secret store backed by macos keychain ([6c194ad](https://github.com/bjornharvold/curlybrace/commit/6c194ad23dfad98a442cfcb0cd7ac60b2dc8065d))
* **core:** add ssh tunnel with host key verification ([8b50dd2](https://github.com/bjornharvold/curlybrace/commit/8b50dd23f3052d887e9638ac1795404d74a4ae54))
* **core:** build driver client options for auth, tls and tunnel ([fe90aac](https://github.com/bjornharvold/curlybrace/commit/fe90aac4ca23c85f33330f13d4805579bd91d35b))
* **core:** classify authentication failures and reject anonymous access to auth servers ([244f28c](https://github.com/bjornharvold/curlybrace/commit/244f28c633da88fbe7dc54ea40c6c178f624bae9))
* **core:** convert documents to and from extended json ([a9a1e88](https://github.com/bjornharvold/curlybrace/commit/a9a1e88c57230279ea53bd05a9bbc98fd6a70ee5))
* **core:** parse filter, sort and projection text into queries ([cbe3cdd](https://github.com/bjornharvold/curlybrace/commit/cbe3cddf597edfb85220fdc2e99e88cf9f284fd2))
* **core:** wire ssh tunnel into connect and disconnect ([115a6cd](https://github.com/bjornharvold/curlybrace/commit/115a6cd10976721de96004ae2ef0c842b1dabd63))
* **ui:** add app state, reducer, persistence and host-key prompt ([a0f7e6b](https://github.com/bjornharvold/curlybrace/commit/a0f7e6b48a00477c6f2838a5aa6a0217572f8a8f))
* **ui:** add connection manager and view-model helpers ([fc9eb51](https://github.com/bjornharvold/curlybrace/commit/fc9eb51a3fc928773bf6ba4ed84140b7ecafd492))
* **ui:** add document tree flattening, delete confirm and editor view-models ([5ecfdd9](https://github.com/bjornharvold/curlybrace/commit/5ecfdd96d322a427382e68137f210b2f824927dd))
* **ui:** add i18n layer and english locale ([ef7769d](https://github.com/bjornharvold/curlybrace/commit/ef7769d1130a0289015eab0b18856639d68c97db))
* **ui:** add sidebar and find bar ([88e7dae](https://github.com/bjornharvold/curlybrace/commit/88e7dae88f36073db8d5bd9b138dad2edc8d87c7))
* **ui:** add tree, table and json views with document editor ([c8bca91](https://github.com/bjornharvold/curlybrace/commit/c8bca917112e69b95aeb63059329695602a6b890))
* **ui:** confirm before deleting a saved connection ([3582fb8](https://github.com/bjornharvold/curlybrace/commit/3582fb811ae4b953423ba8073e464a80f7746904))
* **ui:** log failure details and show safe error details ([6c715f4](https://github.com/bjornharvold/curlybrace/commit/6c715f4297800c2dde1b8ffc44e292b79316ffad))


### Bug Fixes

* **core:** align bson with driver version ([dd9a595](https://github.com/bjornharvold/curlybrace/commit/dd9a5958fedcc96bc4925b2d9a7e7990453a7906))
* **core:** close the check-then-send gap for data answers ([4e41ea1](https://github.com/bjornharvold/curlybrace/commit/4e41ea1860dc333b09eedb201f2a8e661b21134d))
* **core:** drop results of superseded sessions and type host-key/tls errors ([529e94c](https://github.com/bjornharvold/curlybrace/commit/529e94c33b3e48dbf69a01882779c321eb98643f))
* **core:** epoch-stamp connect outcomes and report replaced connects as superseded ([16f5896](https://github.com/bjornharvold/curlybrace/commit/16f5896912d11f4f28a001cd31e2d3b8d0121ce3))
* **core:** forward gssapi feature and reject srv uris with tunnel ([30deced](https://github.com/bjornharvold/curlybrace/commit/30decedba4aa71f947fadd7817c1a1ce476a21d6))
* **core:** gate host-key prompts of replaced connects ([128ad85](https://github.com/bjornharvold/curlybrace/commit/128ad8542fc27bf738a3c51f84654c8d10d111f6))
* **core:** give Connect its own timeout so data commands keep the short one ([97ba647](https://github.com/bjornharvold/curlybrace/commit/97ba647d4525651632b6e113b7ac4f33d5381f48))
* **core:** keep the tunnel accepting after transient accept errors ([dc2db7e](https://github.com/bjornharvold/curlybrace/commit/dc2db7ed9984502ceecdbd8d77344843f2089f7b))
* **core:** never fall back to a relative known_hosts path ([e28d5d1](https://github.com/bjornharvold/curlybrace/commit/e28d5d153b9a8c41131aad6480cc0294ddff11f0))
* **core:** order connect/disconnect by request and cover runtime races ([03bc2bc](https://github.com/bjornharvold/curlybrace/commit/03bc2bc292b13141fa61d9fbbdb79745bea2d05f))
* **core:** reject verified tls through an ssh tunnel with a clear error ([0b1cf3a](https://github.com/bjornharvold/curlybrace/commit/0b1cf3ad2476f2da237a676f5210e1feaa245623))
* **core:** stamp real connect failures with their epoch ([a70b2c8](https://github.com/bjornharvold/curlybrace/commit/a70b2c8641046f7a05581f0baac1fd37815b6e2f))
* **site:** fit the table illustration in the hero ([42afe3b](https://github.com/bjornharvold/curlybrace/commit/42afe3b22918ca0f99b24a306a774af1829d7311))
* **site:** shorten _id cells so the table illustration fits ([d8ce761](https://github.com/bjornharvold/curlybrace/commit/d8ce761f2d02819dd532c7b68458a4ffe909a483))
* **ui:** decide connect outcomes by runtime epoch instead of answer counting ([99da26f](https://github.com/bjornharvold/curlybrace/commit/99da26ff24be940c3b86b8f1ec966609575c58cb))
* **ui:** extract pure click/intent logic and fix sidebar row heights ([17bfaf7](https://github.com/bjornharvold/curlybrace/commit/17bfaf7c2b6bc36967ba317efe3881e198935844))
* **ui:** harden session handling, persistence, secret deletion and host-key flow ([149f975](https://github.com/bjornharvold/curlybrace/commit/149f9756179952d29978558452cd264ea7afbf0f))
* **ui:** ignore connect outcomes older than the latest requested epoch ([7e0b336](https://github.com/bjornharvold/curlybrace/commit/7e0b33660f6da6bb9c0726ea4dcbfe836727a11c))
* **ui:** keep unsaved editor text when the modal is dismissed ([f977ee4](https://github.com/bjornharvold/curlybrace/commit/f977ee4d209b1bfbbca17ecb86bd48c1304f31b0))
* **ui:** key document caches and delete confirm by page identity, widen json lines ([0e44616](https://github.com/bjornharvold/curlybrace/commit/0e44616bea8730afc2eb75ae80f63ccf9d7ea305))
* **ui:** make page key content-sensitive and sync it once per frame ([362852d](https://github.com/bjornharvold/curlybrace/commit/362852da898ce1073bd35c54512341d0885694c8))
* **ui:** make projected pages read-only for edit and clone ([c11ee1f](https://github.com/bjornharvold/curlybrace/commit/c11ee1f72975747c6a05716a717cf95f17bc7c97))
* **ui:** never delete stored secrets on untouched edit ([3073f4a](https://github.com/bjornharvold/curlybrace/commit/3073f4ac601e70e682670525064a5193de38b1e2))

## Changelog

All notable changes to Curlybrace are recorded here. Versions follow
[Semantic Versioning](https://semver.org/). This file is maintained by
[release-please](https://github.com/googleapis/release-please): entries are generated from
[Conventional Commits](https://www.conventionalcommits.org/) when a release pull request is merged,
so please do not edit released sections by hand.

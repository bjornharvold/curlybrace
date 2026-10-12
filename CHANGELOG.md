# Changelog

## 0.1.0 (2026-10-12)


### Features

* **app:** wire runtime, state reducer and eframe shell ([fd348f7](https://github.com/bjornharvold/the-mongodb-gui/commit/fd348f734af41260ff185e7332cf3120cec87a42))
* **core:** add connection config model and validation ([ec4cf40](https://github.com/bjornharvold/the-mongodb-gui/commit/ec4cf40d40a438c23a835389462c7da550072ff1))
* **core:** add document crud service with id handling ([4e2635c](https://github.com/bjornharvold/the-mongodb-gui/commit/4e2635ccd71ed73346bfa989f62409dec67158a2))
* **core:** add document store trait, paging types and test fixtures ([a909ff8](https://github.com/bjornharvold/the-mongodb-gui/commit/a909ff8dc56494840f35fd279b8ef13bf8892131))
* **core:** add mongo store and async runtime bridge ([7e5f2cf](https://github.com/bjornharvold/the-mongodb-gui/commit/7e5f2cf5fd1cbd2fd746e3e44cd0766dba3f4734))
* **core:** add secret store backed by macos keychain ([6c194ad](https://github.com/bjornharvold/the-mongodb-gui/commit/6c194ad23dfad98a442cfcb0cd7ac60b2dc8065d))
* **core:** add ssh tunnel with host key verification ([8b50dd2](https://github.com/bjornharvold/the-mongodb-gui/commit/8b50dd23f3052d887e9638ac1795404d74a4ae54))
* **core:** build driver client options for auth, tls and tunnel ([fe90aac](https://github.com/bjornharvold/the-mongodb-gui/commit/fe90aac4ca23c85f33330f13d4805579bd91d35b))
* **core:** classify authentication failures and reject anonymous access to auth servers ([244f28c](https://github.com/bjornharvold/the-mongodb-gui/commit/244f28c633da88fbe7dc54ea40c6c178f624bae9))
* **core:** convert documents to and from extended json ([a9a1e88](https://github.com/bjornharvold/the-mongodb-gui/commit/a9a1e88c57230279ea53bd05a9bbc98fd6a70ee5))
* **core:** parse filter, sort and projection text into queries ([cbe3cdd](https://github.com/bjornharvold/the-mongodb-gui/commit/cbe3cddf597edfb85220fdc2e99e88cf9f284fd2))
* **core:** wire ssh tunnel into connect and disconnect ([115a6cd](https://github.com/bjornharvold/the-mongodb-gui/commit/115a6cd10976721de96004ae2ef0c842b1dabd63))
* **ui:** add app state, reducer, persistence and host-key prompt ([a0f7e6b](https://github.com/bjornharvold/the-mongodb-gui/commit/a0f7e6b48a00477c6f2838a5aa6a0217572f8a8f))
* **ui:** add connection manager and view-model helpers ([fc9eb51](https://github.com/bjornharvold/the-mongodb-gui/commit/fc9eb51a3fc928773bf6ba4ed84140b7ecafd492))
* **ui:** add document tree flattening, delete confirm and editor view-models ([5ecfdd9](https://github.com/bjornharvold/the-mongodb-gui/commit/5ecfdd96d322a427382e68137f210b2f824927dd))
* **ui:** add i18n layer and english locale ([ef7769d](https://github.com/bjornharvold/the-mongodb-gui/commit/ef7769d1130a0289015eab0b18856639d68c97db))
* **ui:** add sidebar and find bar ([88e7dae](https://github.com/bjornharvold/the-mongodb-gui/commit/88e7dae88f36073db8d5bd9b138dad2edc8d87c7))
* **ui:** add tree, table and json views with document editor ([c8bca91](https://github.com/bjornharvold/the-mongodb-gui/commit/c8bca917112e69b95aeb63059329695602a6b890))
* **ui:** confirm before deleting a saved connection ([3582fb8](https://github.com/bjornharvold/the-mongodb-gui/commit/3582fb811ae4b953423ba8073e464a80f7746904))
* **ui:** log failure details and show safe error details ([6c715f4](https://github.com/bjornharvold/the-mongodb-gui/commit/6c715f4297800c2dde1b8ffc44e292b79316ffad))


### Bug Fixes

* **core:** align bson with driver version ([dd9a595](https://github.com/bjornharvold/the-mongodb-gui/commit/dd9a5958fedcc96bc4925b2d9a7e7990453a7906))
* **core:** close the check-then-send gap for data answers ([4e41ea1](https://github.com/bjornharvold/the-mongodb-gui/commit/4e41ea1860dc333b09eedb201f2a8e661b21134d))
* **core:** drop results of superseded sessions and type host-key/tls errors ([529e94c](https://github.com/bjornharvold/the-mongodb-gui/commit/529e94c33b3e48dbf69a01882779c321eb98643f))
* **core:** epoch-stamp connect outcomes and report replaced connects as superseded ([16f5896](https://github.com/bjornharvold/the-mongodb-gui/commit/16f5896912d11f4f28a001cd31e2d3b8d0121ce3))
* **core:** forward gssapi feature and reject srv uris with tunnel ([30deced](https://github.com/bjornharvold/the-mongodb-gui/commit/30decedba4aa71f947fadd7817c1a1ce476a21d6))
* **core:** gate host-key prompts of replaced connects ([128ad85](https://github.com/bjornharvold/the-mongodb-gui/commit/128ad8542fc27bf738a3c51f84654c8d10d111f6))
* **core:** give Connect its own timeout so data commands keep the short one ([97ba647](https://github.com/bjornharvold/the-mongodb-gui/commit/97ba647d4525651632b6e113b7ac4f33d5381f48))
* **core:** keep the tunnel accepting after transient accept errors ([dc2db7e](https://github.com/bjornharvold/the-mongodb-gui/commit/dc2db7ed9984502ceecdbd8d77344843f2089f7b))
* **core:** never fall back to a relative known_hosts path ([e28d5d1](https://github.com/bjornharvold/the-mongodb-gui/commit/e28d5d153b9a8c41131aad6480cc0294ddff11f0))
* **core:** order connect/disconnect by request and cover runtime races ([03bc2bc](https://github.com/bjornharvold/the-mongodb-gui/commit/03bc2bc292b13141fa61d9fbbdb79745bea2d05f))
* **core:** reject verified tls through an ssh tunnel with a clear error ([0b1cf3a](https://github.com/bjornharvold/the-mongodb-gui/commit/0b1cf3ad2476f2da237a676f5210e1feaa245623))
* **core:** stamp real connect failures with their epoch ([a70b2c8](https://github.com/bjornharvold/the-mongodb-gui/commit/a70b2c8641046f7a05581f0baac1fd37815b6e2f))
* **site:** fit the table illustration in the hero ([42afe3b](https://github.com/bjornharvold/the-mongodb-gui/commit/42afe3b22918ca0f99b24a306a774af1829d7311))
* **site:** shorten _id cells so the table illustration fits ([d8ce761](https://github.com/bjornharvold/the-mongodb-gui/commit/d8ce761f2d02819dd532c7b68458a4ffe909a483))
* **ui:** decide connect outcomes by runtime epoch instead of answer counting ([99da26f](https://github.com/bjornharvold/the-mongodb-gui/commit/99da26ff24be940c3b86b8f1ec966609575c58cb))
* **ui:** extract pure click/intent logic and fix sidebar row heights ([17bfaf7](https://github.com/bjornharvold/the-mongodb-gui/commit/17bfaf7c2b6bc36967ba317efe3881e198935844))
* **ui:** harden session handling, persistence, secret deletion and host-key flow ([149f975](https://github.com/bjornharvold/the-mongodb-gui/commit/149f9756179952d29978558452cd264ea7afbf0f))
* **ui:** ignore connect outcomes older than the latest requested epoch ([7e0b336](https://github.com/bjornharvold/the-mongodb-gui/commit/7e0b33660f6da6bb9c0726ea4dcbfe836727a11c))
* **ui:** keep unsaved editor text when the modal is dismissed ([f977ee4](https://github.com/bjornharvold/the-mongodb-gui/commit/f977ee4d209b1bfbbca17ecb86bd48c1304f31b0))
* **ui:** key document caches and delete confirm by page identity, widen json lines ([0e44616](https://github.com/bjornharvold/the-mongodb-gui/commit/0e44616bea8730afc2eb75ae80f63ccf9d7ea305))
* **ui:** make page key content-sensitive and sync it once per frame ([362852d](https://github.com/bjornharvold/the-mongodb-gui/commit/362852da898ce1073bd35c54512341d0885694c8))
* **ui:** make projected pages read-only for edit and clone ([c11ee1f](https://github.com/bjornharvold/the-mongodb-gui/commit/c11ee1f72975747c6a05716a717cf95f17bc7c97))
* **ui:** never delete stored secrets on untouched edit ([3073f4a](https://github.com/bjornharvold/the-mongodb-gui/commit/3073f4ac601e70e682670525064a5193de38b1e2))

## Changelog

All notable changes to The MongoDB GUI are recorded here. Versions follow
[Semantic Versioning](https://semver.org/). This file is maintained by
[release-please](https://github.com/googleapis/release-please): entries are generated from
[Conventional Commits](https://www.conventionalcommits.org/) when a release pull request is merged,
so please do not edit released sections by hand.

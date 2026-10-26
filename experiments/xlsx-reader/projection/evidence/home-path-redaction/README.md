# 测试证据中的本机路径脱敏（Refs #28）

基线 main：`9b40b39481a933a558556790f84374728545ce8b`。在学生指定的两个目录中找到 14 个文件、23 处本机 home 前缀；全部改为 `<HOME>` 占位。projection 下 6 份，core 测试目录下 8 份，其中 3 份属于既有 DOCX/PPTX 元数据记录。它们只替换路径，不改相邻说明或验收结论。

**原文件因含本机路径而改写，原 hash 如下。** 原始 FAIL/PASS、诊断含义、测试预期及错误顺序均保留；本次脱敏不把旧失败变成通过。3 份清单的 5 个当前文件 hash 引用更新为脱敏版本，各清单另存原 hash。`redaction.json` 记录完整映射、字节数、清单来源和本轮验证。此次普通 PR 不改写 main 历史，也不处理 #137 的文件。

| 仓库相对文件 | 替换数量 | 原 hash | 脱敏后 hash |
|---|---:|---|---|
| `crates/traeva-core/tests/fixtures/docx/r2-review.json` | 1 | `3f6d63b4ed1348777ae008579f5685f8467646c41254c0b08f0d4efd7843abec` | `13e96f5a3963181315929c826feac757cdf0239e5412a91c32d7866b3c3609bb` |
| `crates/traeva-core/tests/fixtures/pptx/README.md` | 1 | `8c42d849dc13db0e5c290dbb14edeccc628029caaf09f2a3f913ebd441e4ed9b` | `3d3df33a394699017ab0a48d206542fe22b679de24d85665dface31aaf141b59` |
| `crates/traeva-core/tests/fixtures/pptx/validation.json` | 1 | `da9335cfec5ac341d09a85255f26ff936776c1978f7046f4d567104b801161bf` | `078e880e01616ccfce3b6003a9d48225353c9f59fde8fab3979485bd549d69f2` |
| `crates/traeva-core/tests/xlsx_reader/evidence/core-clippy.txt` | 2 | `85d0ece02ac491b5bea01fc47e60f331281953fead6358389c33e20245a6e11e` | `b2ab7eb0685929bb44518fa33e9312d73e5999cbfe2be64f5a9a73822c20a2f6` |
| `crates/traeva-core/tests/xlsx_reader/evidence/core-tests.txt` | 2 | `2de4c6427b6eda47e35715ebcb1129ebeac575d227712259bbcbeea28236e7ae` | `22ac8d6e83c36a7370598fdb9a58d9fec2dfb0a151977ec3b3c0d8a5b170a730` |
| `crates/traeva-core/tests/xlsx_reader/evidence/first-shim-compile-FAIL.txt` | 3 | `9760c1fdc5e8670439936eebfa9d695725082f8aac5eb7e92ebbf7f648dff206` | `9d3ed076e903c24102053c0d01cb7979dd5c8ca1f2cb5e739b68cef90c24c824` |
| `crates/traeva-core/tests/xlsx_reader/evidence/legacy-shim-clippy.txt` | 1 | `00bac842e5d4d22bd8dac76576ea8e66f9f9259282e35ab46618c402e0522e80` | `c49ca8edacd413a048403821ab0bef3f3e6b8e7684613c1eeb75bbe034037dea` |
| `crates/traeva-core/tests/xlsx_reader/evidence/projection-shim-fixed.txt` | 1 | `6808bbb6c6ace1e81d0a6056b8205b093a7b0f238dbc3d0113541593f8cb250a` | `5522d56d6d568f5389bc7e32f24205e0f1292593d44ba27bff4e407db6940431` |
| `experiments/xlsx-reader/projection/evidence/aligned-clippy.txt` | 1 | `89ff78e503b9ab751f12a06d705ddfaa6dd5385758b59f9111d8698de5867420` | `cce2e34f9a8b2856b697783cd7f7e3ab6f0c77150c7c4e2838c0599fa0f1378a` |
| `experiments/xlsx-reader/projection/evidence/aligned-tests.txt` | 2 | `5fed4689450b2812f355e53d835d5596eb4861cacf5441f6106ed51b6f6ad814` | `e2b0f071cdf768ac94c6c32ecf2f45ec1d096a8a76b8631f1fd8a154d470d21a` |
| `experiments/xlsx-reader/projection/evidence/binding-budget-recheck.txt` | 1 | `16c05a129327f4f60f6967bb1566668de0d7fb0734357b5c82251732db5fafce` | `a78bdb46c2c7cbd70b5bfa2060b7763afdd58a57613ac956040b666238f4c2bc` |
| `experiments/xlsx-reader/projection/evidence/first-A151-compat-FAIL.txt` | 2 | `6f5b3627bce6929272c538ccba880f9fa4d939d61f06b1dfc85bceddbc3ba2a3` | `d2b4789726ef6d0d2bc1c6a4738c55090e68f571a94fec0c83d55c8be4bdc500` |
| `experiments/xlsx-reader/projection/evidence/first-build-FAIL.txt` | 4 | `08770cc181a854cc62720e351bf82f4e1becd9deea952743ee021161bcd141e5` | `74a4e6f706692b8b50b8ff2d7e7d6f6d8a450f740036bc94eaeda948de1e1af1` |
| `experiments/xlsx-reader/projection/evidence/v23-compat-FAIL.txt` | 1 | `710c936fbd64094428abddaf0543358ee89f92594be1e2380f69fb8f66feb318` | `a0e8a8968547306b50b920f6dceb5322b95b40dd57b719dce49ce35e7bdf30ef` |

验证：

- 独立 XLSX core 验证 crate：22 项测试通过；projection：16 项投影测试及 5 项共享 helper 测试通过。共 43 项，均使用 main 已跟踪的锁文件，以 offline --locked 运行。根及两份独立锁文件、源码、fixture 二进制和测试预期未改。
- core 格式检查通过；projection 格式检查 **FAIL 保留**，指出 main 已有 `experiments/xlsx-reader/src/lib.rs` 兼容转接格式问题。该源文件与基线同字节，未在本次路径清理中修格式或扩大代码范围。详见 `projection-fmt.txt` 与 JSON 中的基线核对记录。
- 最初测试准备脚本误把 main 已跟踪的独立锁当成忽略锁，前置断言 FAIL，未启动 cargo 测试。原记录在 `first-test-preparation-FAIL.json`；随后使用现有锁，未修改它们。
- 全部受影响 JSON 语法有效；原内容逐字节仅替换 home 前缀；当前 hash 引用及原 hash 映射一致。目标目录已跟踪文件及本轮新增证据中的已知本机 home/登录名剩余 0，diff 检查通过。

复跑命令：

```sh
cargo test --offline --locked --manifest-path crates/traeva-core/tests/xlsx_reader/Cargo.toml
cargo test --offline --locked --manifest-path experiments/xlsx-reader/projection/Cargo.toml
```

本轮未修改 Reader、生产注册/分派、共享 DTO/schema、资源上限或安全规则；没有模型或 GUI 操作，未 merge PR、未关 Issue。此记录只验证路径改写及相关专项回归，不外推成完整 Reader/Memory/App 接受，也不替代独立复核。

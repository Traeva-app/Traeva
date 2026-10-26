# 移植开发记录（追加）

- 2026-10-13：判定标准先提交 `bd26c74`。初次 generate-lockfile 解析到 6 个更新的传递依赖；未做 OCR 测量，发现后恢复 #215 lock，仅改独立 harness 的包名。先前 14/14 测试仅为开发检查，最终版本重跑。
- 恢复 lock 后第一次测试 13 PASS / 1 FAIL：系统内存 `lifetime_peak >= current` 断言失败，原始失败段保留于 `first-memory-test-failure.txt`。不假定系统两个字段是原子快照。API 将保留 kernel 原值，并以 max(kernel lifetime, current) 提供至少覆盖当前占用的峰值；父侧跨次取最大且退出前检查。
- 环境检查工具 `python3` 是 3.9，没有 `tomllib`；版本比较改用锁文件已有文本。`py_compile` 默认尝试写非工作区系统缓存而失败；后续将字节码缓存显式放在本实验 target 目录。这两项没有执行 OCR，没有读用户文件。
- 隔离 main worktree 创建尚在写文件时，第一次 cherry-pick 过早启动，被 Git 拒绝；没有覆盖任何用户改动。确认 checkout 完成且干净后继续，新增协议文件的 modify/delete 冲突选择保留完整新文件。最终 `e777a91` 相对 main 仅新增 16 文件。
- 最终依赖恢复后，修复内存峰值语义，15/15 core 测试通过；测试驱动补管道 I/O 错误、无效 JSON、解析后时限/取消检查，3/3 纯 driver 测试通过。
- 首次签名迁移矩阵 `9751706`：79 检查 74 PASS / 5 FAIL；s06/s13 英文页缺 4 行，s05 一次少 1 字。后续对照见 DIAGNOSTIC-PROTOCOL.md，保留所有失败，不更改 truth。CPU-only 单变量没有改善，不采用；C 版本查询候选未构建/运行。
- 收窄契约编译：19个core测试通过；首次Clippy因固定chunks_exact(2)提示而失败，改为as_chunks::<2>()；随后19个core测试、Clippy和release通过。driver三个测试此前同源码通过；不会把pdf fixture的过滤数报成执行。
- trace-01受限shell环境下两个签名包均CVPixelBuffer失败，保留该失败；trace-02在正常macOS环境复现16/190且三页输入像素SHA、尺寸与实际显式选项全部相等。结论止于identifier相关包/签名上下文；不宣称内部缓存根因或永久解决。trace是临时诊断构建，正式Vision文件与manifest均恢复，不采用CPU-only。
- 正式metadata脚本首轮保留14/15（s05单字差异）；内审发现两次prepare结果没有进入脚本最终判定，补精确ready/exit/reaped检查和页段检查，再以同正式metadata复测，15/15读取与2/2准备通过。首轮原始两次准备也为ready，但不倒改原记录的判定结构；一次较好结果不覆盖s05已发生的波动。
- e09e764完整94项矩阵的包外旧实现二进制为98be7d0（仅#215退出谓词修复、识别算法未变），SHA见build-validation.json；最初79项及两组paired继续保留此前G构建。新证据只将两处个人home路径规范化为<HOME>，未改识别正文、期望、时间或FAIL；最终manifest基于规范化后的文件。
- 最终出口加固caa5bc3：纯runtime13/13、fmt/clippy/release通过；正式metadata受影响复测两次prepare通过、读取14/15，s05单字波动再次出现；保留前次15/15及当前FAIL，不以重复取好结果。新增页号与保留标记歧义拒绝，不改准备/监测。

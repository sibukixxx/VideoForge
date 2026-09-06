# YMM4 template fixture

`default.ymmp` is a **synthetic** stand-in that follows the prototype contract
(`Remark` = `VF_PROTO_*`). It exercises the template-patch exporter in CI on
every platform, but it is *not* a file saved by YukkuriMovieMaker4.

Phase 0 (design §46) still requires a real template authored in YMM4 on
Windows: open YMM4, add one audio item / one text item per speaker /
optional 立ち絵 items, set their 備考 (Remark) to the prototype names, save
as `templates/ymm4/default.ymmp` in your workspace, then run

```
videoforge export ymm4 generated/<slug>/project.vfp.json
```

and confirm YMM4 opens the result. Replace this fixture with a trimmed copy
of that real template once it exists.

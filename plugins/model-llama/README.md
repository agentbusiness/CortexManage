# model-llama

CortexManage's model plugin (`model@1`): Gemma 4 (tag `office`; E2B `smalloffice` on 8 GB machines)
on upstream llama.cpp's `llama-server`, the same plugin CortexTerm carries. The manager's every
question claims `office` (`plugins/chatflow-manage/backend/data/stages.toml`). When another app on
this computer already serves Gemma, the mesh hands the tag over and that copy is used. It replaced
model-uzu (Qwen 27B). It declares itself the default for every machine (`[backend.machine]`), so a
home that still has the older engine starts this one and keeps the other on standby; with an app
that no longer carries the older engine the frame sets it aside as well.

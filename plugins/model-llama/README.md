# model-llama

CortexManage's model plugin (`model@1`): Gemma 4 (tag `office`; E2B `smalloffice` on 8 GB machines)
on upstream llama.cpp's `llama-server`, the same plugin CortexTerm carries. The manager's every
question claims `office` (`plugins/chatflow-manage/backend/data/stages.toml`). When another app on
this computer already serves Gemma, the mesh hands the tag over and that copy is used. It replaced
model-uzu (Qwen 27B), which the frame now sets aside on homes that still have it.

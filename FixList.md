# FixList for this project

From Date: 2024-12-23

## Things

1. instead xtask with python, refactor...
    due to the conflict of x86_64 crate. There is incomaptible change between 0.14.13 and 0.14.10.
    PolyHAL x86_64 EScape Reason
    x86_64 stack map error
    x86_64 .got section not aligned with 1000, so not loaded correctly.
    DebugConsole::getchar() not block when not receiving a data
2. support Multi Architecture

3. Support SMP(Symmetric Multi Processing)

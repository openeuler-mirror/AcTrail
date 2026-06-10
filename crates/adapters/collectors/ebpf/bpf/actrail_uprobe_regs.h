#ifndef ACTRAIL_UPROBE_REGS_H
#define ACTRAIL_UPROBE_REGS_H

#if defined(__TARGET_ARCH_x86)
struct pt_regs {
    unsigned long r15;
    unsigned long r14;
    unsigned long r13;
    unsigned long r12;
    unsigned long bp;
    unsigned long bx;
    unsigned long r11;
    unsigned long r10;
    unsigned long r9;
    unsigned long r8;
    unsigned long ax;
    unsigned long cx;
    unsigned long dx;
    unsigned long si;
    unsigned long di;
    unsigned long orig_ax;
    unsigned long ip;
    unsigned long cs;
    unsigned long flags;
    unsigned long sp;
    unsigned long ss;
};

#define ACTRAIL_UPROBE_ARG1(ctx) ((ctx)->di)
#define ACTRAIL_UPROBE_ARG2(ctx) ((ctx)->si)
#define ACTRAIL_UPROBE_ARG3(ctx) ((ctx)->dx)
#define ACTRAIL_UPROBE_ARG4(ctx) ((ctx)->cx)
#define ACTRAIL_UPROBE_RET(ctx) ((ctx)->ax)
#define ACTRAIL_UPROBE_RET2(ctx) ((ctx)->dx)
#define ACTRAIL_GO_UPROBE_ARG1(ctx) ((ctx)->ax)
#define ACTRAIL_GO_UPROBE_ARG2(ctx) ((ctx)->bx)
#define ACTRAIL_GO_UPROBE_ARG3(ctx) ((ctx)->cx)
#define ACTRAIL_GO_UPROBE_ARG4(ctx) ((ctx)->di)
#elif defined(__TARGET_ARCH_arm64)
struct pt_regs {
    unsigned long regs[31];
    unsigned long sp;
    unsigned long pc;
    unsigned long pstate;
};

#define ACTRAIL_UPROBE_ARG1(ctx) ((ctx)->regs[0])
#define ACTRAIL_UPROBE_ARG2(ctx) ((ctx)->regs[1])
#define ACTRAIL_UPROBE_ARG3(ctx) ((ctx)->regs[2])
#define ACTRAIL_UPROBE_ARG4(ctx) ((ctx)->regs[3])
#define ACTRAIL_UPROBE_RET(ctx) ((ctx)->regs[0])
#define ACTRAIL_UPROBE_RET2(ctx) ((ctx)->regs[1])
#define ACTRAIL_GO_UPROBE_ARG1(ctx) ((ctx)->regs[0])
#define ACTRAIL_GO_UPROBE_ARG2(ctx) ((ctx)->regs[1])
#define ACTRAIL_GO_UPROBE_ARG3(ctx) ((ctx)->regs[2])
#define ACTRAIL_GO_UPROBE_ARG4(ctx) ((ctx)->regs[3])
#else
#error "unsupported BPF target architecture for AcTrail uprobes"
#endif

#endif

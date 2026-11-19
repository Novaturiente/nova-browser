#!/usr/bin/env python3
"""CPU use of a process and all its descendants over N seconds, in % of one core.
Usage: cpu.py PID SECONDS"""
import os, sys, time

def tree(root):
    kids = {}
    for p in os.listdir("/proc"):
        if not p.isdigit():
            continue
        try:
            stat = open(f"/proc/{p}/stat").read()
        except OSError:
            continue
        ppid = int(stat.rsplit(")", 1)[1].split()[1])
        kids.setdefault(ppid, []).append(int(p))
    out, todo = [], [root]
    while todo:
        p = todo.pop()
        out.append(p)
        todo += kids.get(p, [])
    return out

def ticks(pids):
    total = 0
    for p in pids:
        try:
            f = open(f"/proc/{p}/stat").read().rsplit(")", 1)[1].split()
        except OSError:
            continue
        total += int(f[11]) + int(f[12])  # utime + stime
    return total

pid, secs = int(sys.argv[1]), float(sys.argv[2])
pids = tree(pid)
t0, c0 = time.time(), ticks(pids)
time.sleep(secs)
pids = tree(pid)
c1, t1 = ticks(pids), time.time()
hz = os.sysconf("SC_CLK_TCK")
print(f"{(c1 - c0) / hz / (t1 - t0) * 100:.0f}")

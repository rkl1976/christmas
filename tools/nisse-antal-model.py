#!/usr/bin/env python3
"""Model of how many nisser walk on each screen, to measure how far above the chosen count it gets.

It follows updateNisser / renderNisser in macos/Sources/JuleHygge/Overlay.swift (the same logic as
src/Layer.cs and linux/src/layer/elves.rs): the count per screen with its offset of -2..+2, arriving,
leaving, and walking over to a neighbouring screen. Left out: meetings, the children's play zone and
the magic puff (they only delay a nisse a little).

Lengths are in the app's own units (points / S), so a screen is about 1700-1900 wide.

    python3 tools/nisse-antal-model.py            # the table
    python3 tools/nisse-antal-model.py --hours 40

Variants:
    nu         the code as it is
    uden-udsv  as it is, but with the offset fixed at 0 (shows what crossing alone does)
    tael-alle  as it is, but no nisse is added while the screen shows its count or more, the ones
               on their way out included
    kun-ur     a nisse is added whenever the screen is under its count, but one is only sent away
               when the clock lowers the count, never because one walked in from a neighbour
    rettet     a crossing moves one place from the old screen to the new one at the moment the nisse
               is handed over (no new nisse on the old screen, nobody sent away on the new), it is
               only allowed while neither screen would be more than one off, and a nisse that was
               sent away keeps walking out when it reaches a neighbouring screen
"""
import argparse
import random

DT = 0.1


class Nisse:
    __slots__ = ("x", "dir", "speed", "state", "timer", "entering", "leaving", "crossing")


class Screen:
    def __init__(self, w, count, variant, rnd):
        self.w, self.count, self.variant, self.r = w, count, variant, rnd
        self.nisser = []
        self.offset = 0
        self.change = 90 + rnd.random() * 90
        self.balance = 0          # only used by "rettet": places lent to (-) or borrowed from (+) a neighbour
        self.going = self.coming = 0   # crossings decided but not handed over yet
        self.left = self.right = None
        self.created = 0

    def neighbor(self, d):
        return self.right if d > 0 else self.left

    def target(self):
        off = 0 if self.variant == "uden-udsv" else self.offset
        return max(1, self.count + off) + self.balance

    def add(self, walking_in):
        r, n = self.r, Nisse()
        n.dir = -1 if r.random() < 0.5 else 1
        n.entering = walking_in
        n.x = (-40 if n.dir > 0 else self.w + 40) if walking_in else 60 + r.random() * (self.w - 120)
        n.speed = 22 + r.random() * 18
        n.state, n.timer, n.leaving, n.crossing = 0, 4 + r.random() * 6, False, False
        self.nisser.append(n)
        self.created += 1

    def may_cross(self, d):
        nb = self.neighbor(d)
        if nb is None or self.r.random() >= 0.5:
            return False
        if self.variant != "rettet":
            return True
        if self.balance - self.going <= -1 or nb.balance + nb.coming >= 1 or self.target() - self.going <= 1:
            return False
        self.going += 1
        nb.coming += 1
        return True

    def update(self, t):
        r = self.r
        lowered = False
        self.change -= DT
        if self.change <= 0:
            self.change = 90 + r.random() * 90
            o = self.offset
            if o == 0:
                self.offset = -1 if r.random() < 0.5 else 1
            elif r.random() < 0.7:
                self.offset += -1 if o > 0 else 1
            else:
                self.offset = max(-2, min(2, o + (1 if o > 0 else -1)))
            lowered = self.offset < o
        target = self.target()
        staying = sum(1 for n in self.nisser if not n.leaving)
        full = self.variant == "tael-alle" and len(self.nisser) >= target
        if staying < target and not full:
            arriving = bool(self.nisser) or t > 2
            for _ in range(1 if arriving else target - staying):
                self.add(arriving)
        elif staying > target if self.variant != "kun-ur" else (lowered and staying > 1):
            for n in reversed(self.nisser):
                if not n.leaving and not n.entering and not n.crossing:
                    n.leaving, n.state = True, 0
                    lf, rf = self.left is None, self.right is None
                    n.dir = (-1 if lf else 1) if lf != rf else (-1 if n.x < self.w / 2 else 1)
                    break
        w = self.w
        for n in self.nisser:
            n.timer -= DT
            if n.state == 0:
                n.x += n.dir * n.speed * DT
                if n.entering and 30 < n.x < w - 30:
                    n.entering = False
                if not n.entering and not n.leaving and not n.crossing:
                    if n.x < 30:
                        if self.may_cross(-1):
                            n.crossing = True
                        else:
                            n.x, n.dir = 30, 1
                    if n.x > w - 30:
                        if self.may_cross(1):
                            n.crossing = True
                        else:
                            n.x, n.dir = w - 30, -1
                if n.entering or n.leaving or n.crossing:
                    continue
                if n.timer <= 0:
                    q = r.random()
                    if q < 0.3:
                        n.state, n.timer = 1, 2 + r.random() * 3
                    elif q < 0.5:
                        n.state, n.timer = 2, 2.5
                    else:
                        if q < 0.7:
                            n.dir = -n.dir
                        n.timer = 4 + r.random() * 8
            elif n.leaving:
                n.state = 0
            elif n.timer <= 0:
                n.state, n.timer = 0, 4 + r.random() * 8
                if r.random() < 0.4:
                    n.dir = -n.dir

    def hand_over(self):
        for i in range(len(self.nisser) - 1, -1, -1):
            n = self.nisser[i]
            if not (n.leaving or n.crossing) or -45 <= n.x <= self.w + 45:
                continue
            del self.nisser[i]
            nb = self.neighbor(n.dir)
            if self.variant == "rettet" and n.crossing:
                self.going -= 1
                nb.coming -= 1
                self.balance -= 1
                nb.balance += 1
            if nb is not None:
                keep = self.variant == "rettet" and n.leaving
                n.crossing, n.leaving, n.entering, n.state = False, keep, True, 0
                n.x = -40 if n.dir > 0 else nb.w + 40
                n.timer = 4 + self.r.random() * 6
                nb.nisser.append(n)


def run(widths, count, variant, hours, seed):
    rnd = random.Random(seed)
    screens = [Screen(w, count, variant, rnd) for w in widths]
    for a, b in zip(screens, screens[1:]):
        a.right, b.left = b, a
    steps = int(hours * 3600 / DT)
    per_screen, totals = [], []
    quarters = [0.0] * 4
    for k in range(steps):
        t = k * DT
        for s in screens:
            s.update(t)
        for s in screens:
            s.hand_over()
        if k % 10 == 0:
            tot = 0
            for s in screens:
                on = sum(1 for n in s.nisser if 0 <= n.x <= s.w)   # the ones actually on the picture
                per_screen.append(on)
                tot += on
            totals.append(tot)
            quarters[min(3, k * 4 // steps)] += tot
    per_screen.sort()
    n = len(per_screen)
    q = len(totals) / 4
    return {
        "mean": sum(per_screen) / n,
        "p95": per_screen[int(n * 0.95)],
        "max": per_screen[-1],
        "over": 100 * sum(1 for v in per_screen if v > count) / n,
        "over2": 100 * sum(1 for v in per_screen if v > count + 2) / n,
        "total_mean": sum(totals) / len(totals),
        "total_max": max(totals),
        "drift": [x / q for x in quarters],
        "born_per_hour": sum(s.created for s in screens) / hours,
    }


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--hours", type=float, default=10)
    ap.add_argument("--seed", type=int, default=1)
    ap.add_argument("--variants", default="nu,uden-udsv,rettet")
    ap.add_argument("--screens", default="1,2,3")
    ap.add_argument("--counts", default="1,3,5,12")
    a = ap.parse_args()
    layouts = {1: [1700], 2: [1700, 1900], 3: [1900, 1700, 1900]}
    print(f"{a.hours:g} timer pr. række. 'pr. skærm' er nisser, der ses på én skærm; valgt+2 er det højeste, udsvinget alene må give.")
    print(f"{'skærme':>6} {'valgt':>5} {'variant':>10} | {'snit':>5} {'p95':>4} {'max':>4} {'>valgt':>7} {'>valgt+2':>9} |"
          f" {'i alt snit':>10} {'max':>4} {'valgt i alt':>11} | {'snit pr. fjerdedel af kørslen':>30} | {'nye/time':>8}")
    for screens in [int(x) for x in a.screens.split(",")]:
        widths = layouts[screens]
        for count in [int(x) for x in a.counts.split(",")]:
            for variant in a.variants.split(","):
                if screens == 1 and variant != "nu":
                    continue
                m = run(widths, count, variant, a.hours, a.seed)
                drift = " ".join(f"{x:6.1f}" for x in m["drift"])
                print(f"{screens:>6} {count:>5} {variant:>10} | {m['mean']:5.2f} {m['p95']:>4} {m['max']:>4} {m['over']:6.1f}% {m['over2']:8.1f}% |"
                      f" {m['total_mean']:10.2f} {m['total_max']:>4} {screens * count:>11} | {drift:>30} | {m['born_per_hour']:8.0f}")
        print()


if __name__ == "__main__":
    main()

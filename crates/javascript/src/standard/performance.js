// The scheduler supplies the public clock. A recording host can also provide a
// monotonic clock that advances inside a synchronous JS turn.
((now, traceNow, notify) => {
    const timeline = [];
    const timestamp = value => {
        const number = Number(value);
        if (!Number.isFinite(number) || number < 0) throw new TypeError("Invalid User Timing timestamp");
        return number;
    };
    const named = name => {
        for (let i = timeline.length - 1; i >= 0; i--) {
            const item = timeline[i];
            if (item.entry.entryType === "mark" && item.entry.name === name) return item;
        }
        throw new DOMException("The performance mark does not exist", "SyntaxError");
    };
    const add = (name, entryType, start, end, detail) => {
        const entry = Object.freeze({
            name, entryType, startTime: start.time, duration: end.time - start.time,
            detail: detail === undefined ? null : structuredClone(detail),
        });
        timeline.push({entry, stamp: start.stamp});
        if (notify) notify(name, entryType === "mark", start.stamp, end.stamp);
        return entry;
    };
    const anchor = () => ({time: now(), stamp: traceNow ? traceNow() : 0});
    const absolute = (value, current) => {
        if (typeof value === "string") {
            const {entry, stamp} = named(value);
            return {time: entry.startTime, stamp};
        }
        const time = timestamp(value);
        return {time, stamp: current.stamp + time - current.time};
    };
    const entries = (name, type) => timeline
        .map(item => item.entry)
        .filter(entry => (name === undefined || entry.name === name)
            && (type === undefined || entry.entryType === type))
        .sort((a, b) => a.startTime - b.startTime);
    const clear = (name, type) => {
        const key = name === undefined ? undefined : String(name);
        for (let i = timeline.length - 1; i >= 0; i--) {
            const {entry} = timeline[i];
            if (entry.entryType === type && (key === undefined || key === entry.name)) {
                timeline.splice(i, 1);
            }
        }
    };
    return {
        now,
        mark(name, options) {
            if (!arguments.length) throw new TypeError("A mark name is required");
            name = String(name);
            const current = anchor();
            const at = options?.startTime === undefined
                ? current : absolute(timestamp(options.startTime), current);
            return add(name, "mark", at, at, options?.detail);
        },
        measure(name, startOrOptions, endMark) {
            if (!arguments.length) throw new TypeError("A measure name is required");
            name = String(name);
            const current = anchor();
            let start = absolute(0, current);
            let end = current;
            let detail;
            if (startOrOptions != null && typeof startOrOptions === "object") {
                const {start: from, end: to, duration, detail: custom} = startOrOptions;
                if (endMark !== undefined || (from === undefined && to === undefined
                    && (duration !== undefined || custom !== undefined))
                    || (from !== undefined && to !== undefined && duration !== undefined)) {
                    throw new TypeError("Invalid performance measure options");
                }
                detail = custom;
                if (from !== undefined) start = absolute(from, current);
                if (to !== undefined) end = absolute(to, current);
                if (duration !== undefined) {
                    const length = timestamp(duration);
                    if (from === undefined) start = {time: end.time - length, stamp: end.stamp - length};
                    else end = {time: start.time + length, stamp: start.stamp + length};
                }
            } else {
                if (startOrOptions !== undefined) start = absolute(String(startOrOptions), current);
                if (endMark !== undefined) end = absolute(String(endMark), current);
            }
            return add(name, "measure", start, end, detail);
        },
        getEntries() { return entries(undefined, undefined); },
        getEntriesByName(name, type) {
            return entries(String(name), type === undefined ? undefined : String(type));
        },
        getEntriesByType(type) { return entries(undefined, String(type)); },
        clearMarks(name) { clear(name, "mark"); },
        clearMeasures(name) { clear(name, "measure"); },
    };
})

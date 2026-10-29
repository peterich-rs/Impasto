(() => {
  // node_modules/.pnpm/@solidjs+signals@2.0.0-rc.17/node_modules/@solidjs/signals/dist/prod/core/error.js
  var NotReadyError = class extends Error {
    constructor(r) {
      const o = Error;
      const t2 = o.stackTraceLimit;
      if (t2 !== void 0) o.stackTraceLimit = 0;
      super();
      if (t2 !== void 0) o.stackTraceLimit = t2;
      this.source = r;
    }
  };
  var StatusError = class extends Error {
    constructor(r, o) {
      try {
        var t2 = String(o instanceof Error ? o.message : o);
      } catch {
      }
      super(t2, {
        cause: o
      });
      this.source = r;
    }
  };
  function unwrapStatusError(r) {
    return r instanceof StatusError ? r.cause : r;
  }
  var NoOwnerError = class extends Error {
    constructor() {
      super("");
    }
  };

  // node_modules/.pnpm/@solidjs+signals@2.0.0-rc.17/node_modules/@solidjs/signals/dist/prod/core/constants.js
  var REACTIVE_NONE = 0;
  var REACTIVE_CHECK = 1 << 0;
  var REACTIVE_DIRTY = 1 << 1;
  var REACTIVE_RECOMPUTING_DEPS = 1 << 2;
  var REACTIVE_IN_HEAP = 1 << 3;
  var REACTIVE_IN_HEAP_HEIGHT = 1 << 4;
  var REACTIVE_ZOMBIE = 1 << 5;
  var REACTIVE_DISPOSED = 1 << 6;
  var REACTIVE_JOINED = 1 << 7;
  var REACTIVE_SNAPSHOT_STALE = 1 << 8;
  var REACTIVE_LAZY = 1 << 9;
  var REACTIVE_MANUAL_WRITE = 1 << 10;
  var REACTIVE_FRAME_READ = 1 << 11;
  var REACTIVE_MISSED_WAKE = 1 << 12;
  var REACTIVE_REASK = 1 << 13;
  var REACTIVE_STAGED_READ = 1 << 14;
  var REACTIVE_PROBED = 1 << 15;
  var REACTIVE_LANE_DIRTY = 1 << 16;
  var REACTIVE_LANE_READ = 1 << 17;
  var REACTIVE_SCREEN_READ = 1 << 18;
  var REACTIVE_PROBE_UNANSWERED = 1 << 19;
  var CONFIG_OWNED_WRITE = 1 << 0;
  var CONFIG_NO_SNAPSHOT = 1 << 1;
  var CONFIG_TRANSPARENT = 1 << 2;
  var CONFIG_IN_SNAPSHOT_SCOPE = 1 << 3;
  var CONFIG_CHILDREN_FORBIDDEN = 1 << 4;
  var CONFIG_AUTO_DISPOSE = 1 << 5;
  var CONFIG_SYNC = 1 << 6;
  var CONFIG_STAGED = 1 << 7;
  var CONFIG_HELD = 1 << 8;
  var CONFIG_HELD_FRAME = 1 << 20;
  var CONFIG_INPUTS_PUBLISHED = 1 << 10;
  var CONFIG_OVERRIDE = 1 << 11;
  var CONFIG_SLOT_NODE = 1 << 12;
  var CONFIG_MANUAL_WRITE = 1 << 18;
  var CONFIG_STAGED_ERROR = 1 << 21;
  var CONFIG_GUESS = 1 << 13;
  var CONFIG_VERDICT = 1 << 15;
  var CONFIG_REDERIVE = 1 << 14;
  var CONFIG_LAGS = 1 << 19;
  var CONFIG_AUTHORITATIVE = 1 << 17;
  var CONFIG_HAS_SNAPSHOT = 1 << 9;
  var CONFIG_FRESH_READ = 1 << 16;
  var CONFIG_PROMOTED = 1 << 22;
  var STATUS_NONE = 0;
  var STATUS_PENDING = 1 << 0;
  var STATUS_ERROR = 1 << 1;
  var STATUS_UNINITIALIZED = 1 << 2;
  var CAUGHT_REARMED = 2;
  var EFFECT_RENDER = 1;
  var EFFECT_USER = 2;
  var EFFECT_TRACKED = 3;
  var NOT_PENDING = {};
  var NO_SNAPSHOT = {};
  var SUPPORTS_PROXY = typeof Proxy === "function";
  var defaultContext = {};
  var $REFRESH = Symbol("refresh");

  // node_modules/.pnpm/@solidjs+signals@2.0.0-rc.17/node_modules/@solidjs/signals/dist/prod/core/owner.js
  function markDisposal(e) {
    let n = e.De;
    while (n) {
      n.R |= REACTIVE_ZOMBIE;
      markDisposal(n);
      n = n.pe;
    }
  }
  function dispose(e) {
    e.l &= ~CONFIG_AUTO_DISPOSE;
    unobserved(e);
  }
  var cleanupError;
  function runCleanup(e) {
    try {
      e();
    } catch (e2) {
      cleanupError ||= [e2];
      schedule();
    }
  }
  function rethrowCleanupError(e) {
    const n = cleanupError;
    cleanupError = void 0;
    if (n && !e) throw n[0];
  }
  function disposeChildren(e, n = false, r) {
    const t2 = e.R;
    if (t2 & REACTIVE_DISPOSED) return;
    if (n && !r && e.T !== null && (e.T.Ue !== null || e.T.me !== null)) disposeChildren(e, false, true);
    if (n) {
      e.R = t2 | REACTIVE_DISPOSED;
    }
    if (n && e.X) {
      if (e.T !== null) e.T.Se = null;
      if (e.m & STATUS_PENDING && e.T?.Re != null || t2 & REACTIVE_FRAME_READ) schedule();
    }
    let l = r ? e.T.Ue : e.De;
    if (r) e.T.Ue = null;
    else e.De = null;
    while (l) {
      const e2 = l;
      e2.l &= ~CONFIG_AUTO_DISPOSE;
      deleteFromHeap(e2, dirtyQueue);
      clearDeps(e2);
      l.Ze = l;
      disposeChildren(l, true);
      l = l.pe;
    }
    if (!r) e.ye = 0;
    if (n && !r && !(t2 & REACTIVE_ZOMBIE) && e._parent !== null && !(e._parent.R & REACTIVE_DISPOSED)) {
      const n2 = e.Ze;
      const r2 = e.pe;
      if (n2 !== null) n2.pe = r2;
      else e._parent.De = r2;
      if (r2 !== null) r2.Ze = n2;
      e.Ze = null;
    }
    runDisposal(e, r);
    if (n && e.Je) {
      const n2 = e.Je;
      e.Je = void 0;
      runCleanup(n2);
    }
  }
  function linkChild(e, n) {
    const r = e.De;
    n.Ze = null;
    n.pe = r;
    if (r !== null) r.Ze = n;
    e.De = n;
  }
  function runDisposal(e, n) {
    const r = n ? e.T.me : e.He;
    if (!r) return;
    if (n) e.T.me = null;
    else e.He = null;
    if (Array.isArray(r)) {
      for (let e2 = r.length - 1; e2 >= 0; e2--) {
        runCleanup(r[e2]);
      }
    } else {
      runCleanup(r);
    }
  }
  function childId(e, n) {
    let r = e;
    while (r.l & CONFIG_TRANSPARENT && r._parent) r = r._parent;
    if (r.id != null) return formatId(r.id, n ? r.ye++ : r.ye);
    throw new Error("");
  }
  function getNextChildId(e) {
    return childId(e, true);
  }
  function inheritId(e, n, r) {
    return e?.id ?? (n ? r?.id : r?.id != null ? getNextChildId(r) : void 0);
  }
  function formatId(e, n) {
    const r = n.toString(36), t2 = r.length - 1;
    return e + (t2 ? String.fromCharCode(64 + t2) : "") + r;
  }
  function getOwner() {
    return context;
  }
  function cleanup(e, n = context) {
    if (!n) return e;
    if (!n.He) n.He = e;
    else if (Array.isArray(n.He)) n.He.push(e);
    else n.He = [n.He, e];
    return e;
  }
  function disposeRootSelf(e = true) {
    disposeChildren(this, e);
    if (cleanupError && !globalQueue.q) rethrowCleanupError();
  }
  function createOwner(e) {
    const n = context;
    const r = e?.transparent ?? false;
    const t2 = {
      id: inheritId(e, r, n),
      l: r ? CONFIG_TRANSPARENT : 0,
      // Present from birth: disposeChildren marks death here on roots too, and
      // a first store of a missing property is a shape transition plus a
      // properties-store allocation per root — on a 100×4 table teardown that
      // was 12 KB and most of disposal's time (2026-10-17).
      R: REACTIVE_NONE,
      Qe: true,
      Ye: n?.Qe ? n.Ye : n,
      De: null,
      pe: null,
      Ze: null,
      He: null,
      A: n?.A || defaultContext,
      ye: 0,
      T: null,
      _parent: n,
      dispose: disposeRootSelf
    };
    if (n) linkChild(n, t2);
    return t2;
  }
  function createRoot(e, n) {
    const r = createOwner(n);
    return runWithOwner(r, () => e(disposeRootSelf.bind(r, true)));
  }

  // node_modules/.pnpm/@solidjs+signals@2.0.0-rc.17/node_modules/@solidjs/signals/dist/prod/core/scheduler.js
  var laneReadHook = null;
  var laneStageHook = null;
  var laneOutcomeHook = null;
  var laneWriteHook = null;
  var applyGuessesHook = null;
  var laneSeamsHook = null;
  var laneCorrectionsHook = null;
  var endLanesHook = null;
  var lanesBlockedHook = null;
  var catchHook = null;
  var freshHook = null;
  var hiddenHook = null;
  var boundarySeamHook = null;
  var heldRunHook = null;
  function installBoundaryHooks(e, n, o, t2, l) {
    catchHook = e;
    freshHook = n;
    hiddenHook = o;
    boundarySeamHook = t2;
    heldRunHook = l;
  }
  var storeParkHook = null;
  var storeCommitHook = null;
  var releaseMarksHook = null;
  var releaseAmbientMarksHook = null;
  var observeFlightHook = null;
  var wireExternalSourceHook = void 0;
  var externalUntrackHook = void 0;
  var transactions = [];
  function newTransaction(e, n = null) {
    const o = {
      tn: [],
      un: [[], []],
      En: [],
      cn: null,
      _parent: n,
      v: e,
      he: null,
      tt: 0,
      nt: false,
      en: null,
      ke: false,
      ln: false,
      I: null
    };
    if (!e) transactions.push(o);
    return o;
  }
  var passLane = null;
  function setPassLane(e) {
    passLane = e;
  }
  var flushTransaction = null;
  var passTx = null;
  function joinPassTx(e, n) {
    if (globalQueue.q && !n || flushTransaction !== null) return joinFuture(e);
    if (passTx === null) passTx = resolveTx(e);
    else merge(resolveTx(e), passTx);
    schedule();
  }
  function txOf(e) {
    return e.T.Re = resolveTx(e.T.Re);
  }
  function liveTx(e) {
    e = resolveTx(e);
    return transactions.indexOf(e) !== -1 ? e : null;
  }
  function resolveTx(e) {
    while (e.cn !== null) e = e.cn;
    return e;
  }
  function joinFuture(e) {
    const n = flushTransaction;
    if (n === null) {
      flushTransaction = e === null ? newTransaction(false) : resolveTx(e);
    } else if (e !== null) merge(resolveTx(e), n);
    if (!globalQueue.q) schedule();
  }
  function merge(e, n) {
    e = resolveTx(e);
    n = resolveTx(n);
    if (e === n || e.v || n.v) return;
    e.cn = n;
    n.tt += e.tt;
    n.nt ||= e.nt;
    append(n.tn, e.tn);
    append(n.un[0], e.un[0]);
    append(n.un[1], e.un[1]);
    append(n.En, e.En);
    if (e.I !== null) append(n.I ??= [], e.I);
    transactions.splice(transactions.indexOf(e), 1);
  }
  function blocked(e) {
    const n = judge;
    judge ??= e;
    const o = e.tt !== 0 || blockedBy(e.tn, e) || !!lanesBlockedHook?.(e);
    judge = n;
    return o;
  }
  var judge = null;
  function ownSource(e, n) {
    const o = e.T?.fe;
    if (o === void 0) return true;
    for (const e2 of o) {
      const o2 = e2.T?.Re;
      if (o2 == null || resolveTx(o2) === n) return true;
    }
    return false;
  }
  function blockedBy(e, n, o = false) {
    for (let t2 = 0; t2 < e.length; t2++) {
      const l = e[t2];
      if (!(l.m & STATUS_PENDING) || l.R & REACTIVE_DISPOSED || l.l & CONFIG_GUESS || l.T.Re !== null && txOf(l) !== n || o && !ownSource(l, n)) continue;
      if (l.i === EFFECT_RENDER) {
        if (onScreen(l, judge ?? n)) return true;
        continue;
      }
      for (let e2 = l.o; e2 !== null; e2 = e2.t) {
        const o2 = e2.u;
        if ((o2.i === EFFECT_RENDER && o2.m & STATUS_PENDING || o2.R & REACTIVE_FRAME_READ || o2.l & CONFIG_REDERIVE && o2.m === STATUS_PENDING) && (e2.Te === o2.Ne || o2.T?.F != null) && !(o2.R & REACTIVE_DISPOSED) && onScreen(o2, judge ?? n)) return true;
      }
    }
    return false;
  }
  function onScreen(e, n) {
    return !(e.R & REACTIVE_ZOMBIE && removalStagedBy(e, n)) && !hiddenHook?.(e);
  }
  function removalStagedBy(e, n) {
    let o = e;
    while (o?.R & REACTIVE_ZOMBIE) o = o._parent;
    return o?.l & CONFIG_HELD && o.T?.Re != null && txOf(o) === resolveTx(n);
  }
  var heldTrims = [];
  function heldTrim(e) {
    heldTrims.push(e);
  }
  var deferredZombies = [];
  function deferZombie(e) {
    deleteFromHeap(e, dirtyQueue);
    e.R &= -4;
    deferredZombies.push(e);
  }
  function holdNode(e, n) {
    e.l |= CONFIG_HELD;
    list(e, n);
  }
  function list(e, n) {
    const o = ext(e);
    if (o.Re === n) return;
    o.Re = n;
    n.tn.push(e);
  }
  function holdFrame(e, n) {
    for (let o = e.De; o !== null; o = o.pe) {
      if (o.T?.Re?.v) continue;
      if (o.X !== void 0) {
        if (!(o.l & CONFIG_HELD)) holdNode(o, n);
        o.l |= CONFIG_HELD_FRAME;
      }
      holdFrame(o, n);
    }
  }
  function heldUnder(e, n, o) {
    for (let t2 = e.De; t2 !== null; t2 = t2.pe) {
      if ((t2.l & (CONFIG_HELD | CONFIG_OVERRIDE)) === CONFIG_HELD && txOf(t2) !== n) o(t2);
      heldUnder(t2, n, o);
    }
  }
  function unhold(e) {
    e.T.Re = null;
    e.l &= ~CONFIG_HELD;
  }
  var dirtyQueue = {
    $e: new Array(2e3).fill(void 0),
    p: false,
    Ge: 0,
    et: 0
  };
  var clock = 0;
  var scheduled = false;
  var halted = false;
  var haltNotified = false;
  var syncDepth = 0;
  var actionDepth = 0;
  var question = 0;
  var MAINLINE_QUESTION = 2147483647;
  function schedule() {
    if (halted) {
      notifyHalted();
      return;
    }
    if (scheduled) return;
    scheduled = true;
    if (!syncDepth && !globalQueue.q) {
      queueMicrotask(flush);
    }
  }
  var ROOT_ERROR_HOOK = Symbol.for("solid-js/root-error-hook");
  function haltReactivity(e) {
    if (halted) return;
    halted = true;
    rethrowCleanupError(true);
    let n = "[REACTIVITY_HALTED]";
    const o = e !== void 0 && globalThis.reportError;
    o || e === void 0 ? console.error(n) : console.error(n, e);
    o && o(e);
  }
  function notifyHalted() {
    if (haltNotified) return;
    haltNotified = true;
    console.error("[REACTIVITY_HALTED]");
  }
  var GlobalQueue = class _GlobalQueue {
    un = [[], []];
    q = false;
    flush() {
      if (this.q) return;
      this.q = true;
      resyncUnflushedCompanions();
      passLane = null;
      passTx = null;
      try {
        sweepDormant();
        runHeap(dirtyQueue, _GlobalQueue._e);
        this.settle();
        clock++;
        scheduled = dirtyQueue.et >= dirtyQueue.Ge || pendingNodes.length !== 0;
        this.run(EFFECT_RENDER);
        this.run(EFFECT_USER);
        if (false) ;
        if (false) ;
        if (false) ;
      } finally {
        this.q = false;
        passTx = null;
      }
    }
    /** L2 — the seam: end of the pure phase. Commit this flush's staged nodes,
     * or park them with the transaction the flush joined; then land every
     * transaction no frame is waiting on. The landing flush is an ordinary
     * flush: the settle walk (async.ts) re-ran the parked readers in its pure
     * phase, and here no flight the transaction holds has a frame deriving
     * from it (`blocked`). */
    settle() {
      if (laneCorrectionsHook?.()) runHeap(dirtyQueue, _GlobalQueue._e);
      const e = flushTransaction;
      flushTransaction = null;
      applyGuessesHook?.(e);
      const n = e !== null && !e.v ? resolveTx(e) : null;
      if (n !== null) {
        const e2 = pendingNodes.length;
        for (let o3 = 0; o3 < e2; o3++) {
          const e3 = pendingNodes[o3];
          if (!(e3.l & (CONFIG_HELD | CONFIG_OVERRIDE)) && (e3.T === null || e3.T.Re === null || !e3.T.Re.v)) {
            if (e3.se === NOT_PENDING && e3.m & STATUS_UNINITIALIZED && e3.i !== EFFECT_RENDER) continue;
            if (e3.se !== NOT_PENDING && !(e3.m & (STATUS_PENDING | STATUS_UNINITIALIZED)) && e3.T?.Ue == null && e3.T?.me == null && // The comparator last: a pass that created children (an object
            // owning a fresh async memo, #4111) must not have it read them.
            // A NotReady read there is a change (`isSame`).
            isSame(e3, e3.Ie, e3.se)) {
              e3.se = NOT_PENDING;
              e3.l &= ~CONFIG_STAGED;
              continue;
            }
            holdNode(e3, n);
          }
          if (e3.l & CONFIG_STAGED) holdFrame(e3, n);
        }
        pendingNodes.length = 0;
        storeParkHook?.(n);
        const o2 = deferredZombies.splice(0);
        if (observeFlightHook) {
          o2.sort((e3, n2) => e3.Fe - n2.Fe);
          for (let e3 = 0; e3 < o2.length; e3++) if ((o2[e3].R & (REACTIVE_DISPOSED | REACTIVE_ZOMBIE)) === REACTIVE_ZOMBIE && o2[e3].l & CONFIG_VERDICT) recompute(o2[e3]);
        }
      } else {
        commitPendingNodes();
        for (let e2 = 0; e2 < heldTrims.length; e2++) {
          const n2 = heldTrims[e2];
          if (n2.T?.F == null && !n2.Ae) trimStaleDeps(n2);
        }
      }
      if (heldTrims.length) heldTrims.length = 0;
      const o = this.un;
      const t2 = this.un = [[], []];
      if (laneSeamsHook) {
        laneSeamsHook(n !== null ? stagedReaders : null);
        if (stagedReaders.length) stagedReaders.length = 0;
      }
      this.un = [[], []];
      for (let e2 = transactions.length - 1; e2 >= 0; e2--) {
        const n2 = transactions[e2];
        if (blocked(n2)) continue;
        transactions.splice(e2, 1);
        endLanesHook?.(n2);
        land(n2);
        e2 = transactions.length;
      }
      storeCommitHook?.();
      const l = n && liveTx(n);
      releaseAmbientMarksHook?.(l);
      const s = l !== null;
      for (let e2 = 0; e2 < 2; e2++) {
        if (s) append(l.un[e2], o[e2]);
        this.un[e2] = s || t2[e2].length || this.un[e2].length ? t2[e2].concat(s ? [] : o[e2], this.un[e2]) : o[e2];
      }
      if (deferredZombies.length !== 0) {
        const e2 = deferredZombies.splice(0).sort((e3, n2) => e3.Fe - n2.Fe);
        for (let n2 = 0; n2 < e2.length; n2++) {
          if (!(e2[n2].R & REACTIVE_DISPOSED)) recompute(e2[n2]);
        }
      }
      boundarySeamHook?.();
    }
    run(e) {
      const n = this.un[e - 1];
      if (n.length) {
        this.un[e - 1] = [];
        runQueue(n, e);
      }
    }
    enqueue(e, n, o = passLane) {
      if (e) (o !== null ? o : this).un[e - 1].push(n);
      schedule();
    }
    /** Status reaching the root: pending is absorbed (the root mount defers —
     * dev records it for the ASYNC_OUTSIDE_LOADING_BOUNDARY FYI); an error is
     * unhandled (`false` → the caller halts).
     *
     * L2: this is where a transaction is registered. Pending that reaches an
     * observer (a render effect: the frame) is a frame that cannot show — the
     * flush parks, and the transaction waits on the flight while a frame
     * derives from it (`blocked`). Pending nobody observes holds nothing: a
     * memo in flight that no frame reads is not a frame waiting. (Base: the
     * queue's `_asyncReporters`, INV-3's one registration site.)
     *
     * A boundary between the observer and the root (boundaries.ts) is asked
     * first: a fallback catches what is not ready under it, and the root
     * never hears of it. */
    notify(e, n, o, t2) {
      if (catchHook?.(e, o & n, t2)) return true;
      if (n & STATUS_PENDING) {
        if (o & STATUS_PENDING) {
          if (passLane === null) joinFuture(null);
        }
        return true;
      }
      return false;
    }
  };
  function reruns(e) {
    for (let n = 0; n < e.En.length; n++) {
      const o = e.En[n];
      if (o.R & REACTIVE_DISPOSED || o.m & STATUS_ERROR) continue;
      if (o.R & REACTIVE_FRAME_READ || o.l & CONFIG_VERDICT) {
        o.R &= ~REACTIVE_FRAME_READ;
        o.Ae = false;
        for (let e2 = o.De; e2 !== null; e2 = e2.pe) if (e2.i) e2.Ae = false;
        enqueueSub(o);
      }
    }
    e.En.length = 0;
  }
  function land(e) {
    reruns(e);
    if (e.I !== null) releaseMarksHook(e.I);
    for (let n = 0; n < e.tn.length; n++) {
      const o = e.tn[n];
      if (o.T.Re === null || txOf(o) !== e) continue;
      o.T.Re = null;
      commitPendingNode(o);
    }
    if (flushTransaction === e) flushTransaction = null;
    releaseQueues(e);
  }
  function releaseQueues(e) {
    for (let n = 0; n < 2; n++) {
      append(globalQueue.un[n], e.un[n]);
      e.un[n] = [];
    }
  }
  function append(e, n) {
    for (let o = 0; o < n.length; o++) e.push(n[o]);
  }
  var pendingNodes = [];
  var stagedReaders = [];
  function staleReader(e, n) {
    if (!(e.R & REACTIVE_FRAME_READ)) {
      e.R |= REACTIVE_FRAME_READ;
      n.En.push(e);
    }
  }
  function queuePendingNode(e) {
    pendingNodes.push(e);
    if (!globalQueue.q) markUnflushedStaged();
  }
  var notifyEpoch = 0;
  function bumpNotifyEpoch() {
    notifyEpoch++;
  }
  function insertSubs(e) {
    e.Me = notifyEpoch;
    const n = e.l;
    const o = (n & CONFIG_HAS_SNAPSHOT) !== 0 && e.T?.Pe !== void 0;
    for (let n2 = e.o; n2 !== null; n2 = n2.t) {
      const e2 = n2.u;
      if (e2.R & REACTIVE_RECOMPUTING_DEPS && n2.Te === e2.Ne && n2 !== e2._) e2.R |= REACTIVE_MISSED_WAKE;
      if (o && e2.l & CONFIG_IN_SNAPSHOT_SCOPE) {
        e2.R |= REACTIVE_SNAPSHOT_STALE;
        continue;
      }
      enqueueSub(e2);
    }
  }
  function laneDirty(e, n) {
    for (let o = e.o; o !== null; o = o.t) {
      const e2 = o.u.T?.Re;
      if (e2?.v ? sameLane(e2, n) : o.u.i) o.u.R |= REACTIVE_LANE_DIRTY;
    }
  }
  function sameLane(e, n) {
    return e === n || e.en !== null && e.en === n.en;
  }
  function commitPendingNode(e) {
    const n = e;
    if (!n.X) {
      e.l &= -385;
      if (e.se !== NOT_PENDING) {
        e.Ie = e.se;
        e.se = NOT_PENDING;
      }
      return;
    }
    const o = e.se === NOT_PENDING && (n.m & STATUS_PENDING) !== 0;
    const t2 = e.l & CONFIG_STAGED_ERROR;
    if (!o && n.T) n.T.de = void 0;
    e.l = o ? e.l & -1048833 | CONFIG_INPUTS_PUBLISHED : e.l & -3147137;
    if (t2) {
      const o2 = e.se;
      e.se = NOT_PENDING;
      e.ie = clock + 1;
      notifyStatus(n, STATUS_ERROR, o2);
    } else if (e.se !== NOT_PENDING) {
      e.Ie = e.se;
      e.se = NOT_PENDING;
      n.m &= ~STATUS_UNINITIALIZED;
      if (n.T != null) n.T.Ve = false, n.T.je = NOT_PENDING;
      if (e.i && e.i !== EFFECT_TRACKED) {
        e.Ae = true;
        globalQueue.enqueue(e.i, e.W ??= GlobalQueue.K.bind(null, n));
      }
    }
    n.ce = false;
    n.R &= ~REACTIVE_MANUAL_WRITE;
    if (n.T?.F == null) trimStaleDeps(n);
    if (!(n.m & STATUS_PENDING)) n.m &= ~STATUS_UNINITIALIZED;
    if (!o && n.T != null && (n.T.Ue !== null || n.T.me !== null)) GlobalQueue.Ce(n, false, true);
  }
  function commitPendingNodes() {
    for (let e = 0; e < pendingNodes.length; e++) {
      const n = pendingNodes[e];
      if (!(n.l & (CONFIG_OVERRIDE | CONFIG_HELD))) commitPendingNode(n);
    }
    pendingNodes.length = 0;
  }
  var globalQueue = new GlobalQueue();
  function flush(e) {
    if (actionDepth > 0) {
      return e ? e() : void 0;
    }
    if (e) {
      syncDepth++;
      try {
        return e();
      } finally {
        try {
          flush();
        } finally {
          syncDepth--;
        }
      }
    }
    if (globalQueue.q) {
      return;
    }
    if (halted) return;
    while (scheduled) {
      globalQueue.flush();
    }
    rethrowCleanupError();
  }
  function runQueue(e, n) {
    for (let o = 0; o < e.length; o++) e[o](n);
  }

  // node_modules/.pnpm/@solidjs+signals@2.0.0-rc.17/node_modules/@solidjs/signals/dist/prod/core/heap.js
  function enqueueSub(e) {
    if (dirtyQueue.Ge > e.Fe) dirtyQueue.Ge = e.Fe;
    insertIntoHeap(e, dirtyQueue);
  }
  function actualInsertIntoHeap(e, t2) {
    const E = (e._parent?.Qe ? e._parent.Ye?.Fe : e._parent?.Fe) ?? -1;
    if (E >= e.Fe) e.Fe = E + 1;
    const n = e.Fe;
    const I = t2.$e[n];
    if (I === void 0) t2.$e[n] = e;
    else {
      const t3 = I.We;
      t3.be = e;
      e.We = t3;
      I.We = e;
    }
    if (n > t2.et) t2.et = n;
  }
  function insertIntoHeap(e, t2) {
    let E = e.R;
    if (E & (REACTIVE_IN_HEAP | REACTIVE_RECOMPUTING_DEPS)) return;
    if (E & REACTIVE_CHECK) {
      e.R = E & -4 | REACTIVE_DIRTY | REACTIVE_IN_HEAP;
    } else {
      e.R = E | REACTIVE_IN_HEAP;
      if (t2.p) markNode(e);
    }
    if (!(E & REACTIVE_IN_HEAP_HEIGHT)) actualInsertIntoHeap(e, t2);
  }
  function insertIntoHeapHeight(e, t2) {
    let E = e.R;
    if (E & (REACTIVE_IN_HEAP | REACTIVE_RECOMPUTING_DEPS | REACTIVE_IN_HEAP_HEIGHT)) return;
    e.R = E | REACTIVE_IN_HEAP_HEIGHT;
    actualInsertIntoHeap(e, t2);
  }
  function deleteFromHeap(e, t2) {
    const E = e.R;
    if (!(E & (REACTIVE_IN_HEAP | REACTIVE_IN_HEAP_HEIGHT))) return;
    e.R = E & -25;
    const n = e.Fe;
    if (e.We === e) t2.$e[n] = void 0;
    else {
      const E2 = e.be;
      const I = t2.$e[n];
      const i = E2 ?? I;
      if (e === I) t2.$e[n] = E2;
      else e.We.be = E2;
      i.We = e.We;
    }
    e.We = e;
    e.be = void 0;
  }
  function markHeap(e) {
    if (e.p) return;
    e.p = true;
    for (let t2 = 0; t2 <= e.et; t2++) {
      for (let E = e.$e[t2]; E !== void 0; E = E.be) {
        if (E.R & REACTIVE_IN_HEAP) markNode(E);
      }
    }
  }
  function markNode(e, t2 = REACTIVE_DIRTY) {
    const E = e.R;
    if ((E & (REACTIVE_CHECK | REACTIVE_DIRTY)) >= t2) return;
    e.R = E & -4 | t2;
    for (let t3 = e.o; t3 !== null; t3 = t3.t) {
      markNode(t3.u, REACTIVE_CHECK);
    }
  }
  function runHeap(e, t2) {
    e.p = false;
    for (e.Ge = 0; e.Ge <= e.et; e.Ge++) {
      let E = e.$e[e.Ge];
      while (E !== void 0) {
        if (E.R & REACTIVE_IN_HEAP) t2(E);
        else adjustHeight(E, e);
        E = e.$e[e.Ge];
      }
    }
    e.et = 0;
  }
  function adjustHeight(e, t2) {
    deleteFromHeap(e, t2);
    let E = e.Fe;
    for (let t3 = e.C; t3; t3 = t3.G) {
      const e2 = t3.S;
      if (e2.X && e2.Fe >= E) E = e2.Fe + 1;
    }
    if (e.Fe !== E) {
      e.Fe = E;
      for (let E2 = e.o; E2 !== null; E2 = E2.t) insertIntoHeapHeight(E2.u, t2);
    }
  }

  // node_modules/.pnpm/@solidjs+signals@2.0.0-rc.17/node_modules/@solidjs/signals/dist/prod/core/graph.js
  var slotUnobservedHook;
  function unlinkSubs(e) {
    const n = e.S;
    const l = e.G;
    const o = e.t;
    const s = e.Xe;
    if (o !== null) o.Xe = s;
    else n.ve = s;
    if (s !== null) s.t = o;
    else {
      n.o = o;
      if (o === null) {
        if (n.l & CONFIG_SLOT_NODE) slotUnobservedHook(n);
        else if (n.T?.we) runCleanup(n.T.we);
        const e2 = n;
        e2.X && e2.l & CONFIG_AUTO_DISPOSE && !(e2.R & REACTIVE_ZOMBIE) && !(e2.m & STATUS_PENDING) && (deferredReleases ? deferredReleases.push(e2) : unobserved(e2));
      }
    }
    return l;
  }
  function trimStaleDeps(e) {
    const n = e._;
    let l = n !== null ? n.G : e.C;
    if (l !== null) {
      do {
        l = unlinkSubs(l);
      } while (l !== null);
      if (n !== null) n.G = null;
      else e.C = null;
    }
  }
  function clearDeps(e) {
    let n = e.C;
    if (!n) return;
    do {
      n = unlinkSubs(n);
    } while (n !== null);
    e.C = null;
    e._ = null;
  }
  var deferredReleases = null;
  var deferReleases = (e) => deferredReleases = e;
  function unobserved(e) {
    deleteFromHeap(e, dirtyQueue);
    clearDeps(e);
    disposeChildren(e, true);
  }
  var dormantNodes = /* @__PURE__ */ new Set();
  function sweepDormant() {
    if (dormantNodes.size === 0) return;
    for (const e of dormantNodes) {
      if (!e.o && e.l & CONFIG_AUTO_DISPOSE && !(e.m & STATUS_PENDING) && !(e.R & (REACTIVE_DISPOSED | REACTIVE_ZOMBIE))) {
        unobserved(e);
      }
    }
    dormantNodes.clear();
  }
  function link(e, n) {
    const l = n._;
    if (l !== null && l.S === e) return;
    let o = null;
    const s = n.R & REACTIVE_RECOMPUTING_DEPS;
    if (s) {
      o = l !== null ? l.G : n.C;
      if (o !== null && o.S === e) {
        o.Te = n.Ne;
        n._ = o;
        return;
      }
    }
    const r = e.ve;
    if (r !== null && r.u === n && (!s || r.Te === n.Ne)) return;
    const t2 = n._ = e.ve = {
      S: e,
      u: n,
      G: o,
      Xe: r,
      t: null,
      Te: n.Ne
    };
    if (l !== null) l.G = t2;
    else n.C = t2;
    if (r !== null) r.t = t2;
    else e.o = t2;
    bumpNotifyEpoch();
  }

  // node_modules/.pnpm/@solidjs+signals@2.0.0-rc.17/node_modules/@solidjs/signals/dist/prod/core/async.js
  function addPendingSource(e, t2) {
    if (e.T?.fe?.has(t2)) return false;
    (ext(e).fe ??= /* @__PURE__ */ new Set()).add(t2);
    return true;
  }
  function removePendingSource(e, t2) {
    const n = e.T?.fe;
    if (!n?.delete(t2)) return false;
    if (!n.size) e.T.fe = void 0;
    return true;
  }
  function clearPendingSources(e) {
    if (e.T !== null) e.T.fe = void 0;
  }
  function retryReaches(e, t2) {
    for (let n = e.C; n; n = n.G) {
      const e2 = n.S;
      if (e2 === t2 || e2.T?.fe?.has(t2)) return true;
    }
    return false;
  }
  function parkLoadingWindow(e, t2) {
    ext(e).ue = true;
    if (t2.source) addPendingSource(e, t2.source);
    if (!(e.m & STATUS_ERROR)) setPendingError(e, t2.source, t2);
  }
  function setPendingError(e, t2, n) {
    if (!t2) {
      if (e.T !== null) e.T.F = null;
      return;
    }
    if (n instanceof NotReadyError && n.source === t2) {
      ext(e).F = n;
      return;
    }
    const r = e.T?.F;
    if (!(r instanceof NotReadyError) || r.source !== t2) {
      ext(e).F = new NotReadyError(t2);
    }
  }
  function forEachDependent(e, t2) {
    for (let n = e.o; n !== null; n = n.t) t2(n.u, n);
  }
  function releaseIfSettledUnobserved(e) {
    e.X && e.l & CONFIG_AUTO_DISPOSE && !e.o && !(e.R & REACTIVE_ZOMBIE) && !(e.m & STATUS_PENDING) && unobserved(e);
  }
  function releaseSettledDependents(e) {
    let t2;
    const n = /* @__PURE__ */ new Set();
    const visit = (e2) => {
      if (n.has(e2)) return;
      n.add(e2);
      if (!e2.o && e2.l & CONFIG_AUTO_DISPOSE) (t2 ??= []).push(e2);
      forEachDependent(e2, visit);
    };
    forEachDependent(e, visit);
    if (t2) for (const e2 of t2) releaseIfSettledUnobserved(e2);
  }
  function stageError(e, t2, n) {
    if (!(e.l & CONFIG_HELD || n && e.R & REACTIVE_STAGED_READ && globalQueue.q && !e.fam) || passLane) return false;
    clearPendingSources(e);
    if (e.T) e.T.ue = false;
    if (!((e.m &= STATUS_ERROR | STATUS_UNINITIALIZED) & STATUS_ERROR)) setPendingError(e);
    e.se = t2 instanceof StatusError ? t2 : new StatusError(e, t2);
    e.l |= CONFIG_STAGED | CONFIG_STAGED_ERROR;
    settlePendingSource(e);
    schedule();
    return true;
  }
  function settleErroredDependents(e, t2) {
    let n = false;
    const r = /* @__PURE__ */ new Set();
    const visit = (e2) => {
      if (r.has(e2)) return;
      r.add(e2);
      if (e2.T?.F === t2) {
        enqueueSub(e2);
        n = true;
      }
      forEachDependent(e2, visit);
    };
    forEachDependent(e, visit);
    if (n) schedule();
  }
  function settlePendingSource(e, t2 = e) {
    removePendingSource(e, t2);
    let n = false;
    let r;
    const o = /* @__PURE__ */ new Set();
    const settle = (f) => {
      if (o.has(f) || // A conditional dropped this source, but another dependency can still
      // carry it. Only retire pending state inherited through the recovered
      // branch. Deliberately NOT marked visited on this early return: the
      // carrying dependency may itself be a later branch of this same walk
      // (two unchanged memos converging), and its visit must be free to
      // re-examine this node once that branch has retired the source.
      t2 !== e && retryReaches(f, t2) || !removePendingSource(f, t2))
        return f.l & CONFIG_VERDICT && enqueueSub(f);
      o.add(f);
      f.ie = clock;
      const u = f.T?.fe?.values().next().value;
      const s = f.m & STATUS_ERROR;
      if (u) {
        if (!s) setPendingError(f, u);
      } else {
        f.m &= ~STATUS_PENDING;
        if (!s) setPendingError(f);
        if (f.l & CONFIG_HELD) joinFuture(txOf(f));
        if (f.T?.ue) {
          enqueueSub(f);
          n = true;
        }
        if (f.T !== null) f.T.ue = false;
        if (!f.o && f.l & CONFIG_AUTO_DISPOSE) (r ??= []).push(f);
      }
      forEachDependent(f, settle);
    };
    forEachDependent(e, settle);
    if (r) for (const e2 of r) releaseIfSettledUnobserved(e2);
    if (n) schedule();
  }
  function isThenable(e) {
    return e != null && typeof e === "object" && typeof e.then === "function";
  }
  function releaseFlightTeardown(e) {
    const t2 = e.T?.ae;
    if (t2 != null) {
      e.T.ae = null;
      t2();
    }
  }
  function handleAsync(e, t2, n, r) {
    let o = false;
    let f = false;
    if (typeof t2 === "object" && t2 !== null) {
      untrack(() => {
        o = t2[Symbol.asyncIterator];
        f = !o && isThenable(t2);
      });
    }
    if (!f && !o) {
      if (e.T !== null) e.T.Se = null;
      e.ce = false;
      return t2;
    }
    ext(e).Se = t2;
    e.T.Ee = question || MAINLINE_QUESTION;
    e.T.fe = void 0;
    let u;
    const handleError = (o2) => {
      if (e.T?.Se !== t2) return;
      let f2 = o2 instanceof NotReadyError;
      if (f2 && e.ce) {
        if (e.T !== null) e.T.Se = null;
        parkLoadingWindow(e, o2);
        e.ie = clock;
        return;
      }
      if (f2 || n || !stageError(e, o2)) notifyStatus(e, f2 ? STATUS_PENDING : STATUS_ERROR, o2);
      if (f2) settlePendingSource(e);
      e.ie = clock;
      if (!f2) releaseSettledDependents(e);
      r?.(o2, f2);
    };
    const asyncWrite = (r2, o2) => {
      if (e.T?.Se !== t2) return;
      if (e.R & REACTIVE_DIRTY) return;
      const f2 = e.m & STATUS_UNINITIALIZED;
      landStatus(e);
      if (n) {
        try {
          n(r2);
        } catch (e2) {
          handleError(e2);
          return;
        }
        if (!(e.l & CONFIG_HELD)) e.T.de = void 0;
        if (f2) landStatus(e, true);
      } else {
        setSignal(e, () => r2);
      }
      if (e.se === NOT_PENDING) {
        e.ce = false;
        trimStaleDeps(e);
      }
      settlePendingSource(e);
      schedule();
      flush();
      o2?.();
    };
    const settleAutodispose = () => {
      if (e.l & CONFIG_AUTO_DISPOSE && !e.o && !(e.m & STATUS_PENDING)) {
        unobserved(e);
        return true;
      }
      return false;
    };
    const consumeIterator = (n2, r2) => {
      const o2 = n2[Symbol.asyncIterator]();
      let f2 = false;
      let s2 = false;
      let i = !r2;
      const close = () => {
        if (s2) return;
        s2 = true;
        try {
          const e2 = o2.return?.();
          if (isThenable(e2)) e2.then(void 0, () => {
          });
        } catch {
        }
      };
      cleanup(close, r2);
      ext(e).ae = close;
      const iterateOrRelease = () => {
        try {
          if (!settleAutodispose()) iterate();
        } catch (e2) {
          handleError(e2);
        }
      };
      const iterate = () => {
        let n3, r3, l2 = false, a = false, S = true;
        const c = o2.next();
        const E = isThenable(c) ? c : {
          then: (e2) => void e2(c)
        };
        E.then((r4) => {
          if (S && i) {
            n3 = r4;
            l2 = true;
            if (r4.done) s2 = true;
          } else if (e.T?.Se !== t2) {
            return;
          } else if (!r4.done) {
            f2 = true;
            asyncWrite(r4.value, iterateOrRelease);
          } else {
            s2 = true;
            if (f2) {
              schedule();
              flush();
            } else {
              asyncWrite(void 0);
            }
            settleAutodispose();
          }
        }, (n4) => {
          if (S && i) {
            r3 = n4;
            a = true;
          } else if (e.T?.Se === t2) {
            s2 = true;
            handleError(n4);
            settleAutodispose();
          }
        });
        S = false;
        if (a) {
          s2 = true;
          handleError(r3);
          if (i) throw r3;
          return true;
        }
        if (l2 && !n3.done) {
          u = n3.value;
          f2 = true;
          return iterate();
        }
        return l2 && n3.done;
      };
      const l = iterate();
      i = false;
      return f2 || l;
    };
    let s = null;
    const flattenIfIterable = (e2, t3) => {
      let n2 = false;
      if (typeof e2 === "object" && e2 !== null) {
        untrack(() => {
          n2 = e2[Symbol.asyncIterator];
        });
      }
      if (!n2) return false;
      u = void 0;
      const r2 = consumeIterator(e2, t3);
      if (!t3) s = r2;
      return true;
    };
    if (f) {
      let n2 = false, r2 = false, o2, f2 = true;
      t2.then((r3) => {
        if (f2) {
          u = r3;
          n2 = true;
        } else {
          try {
            if (e.T?.Se === t2 && !(e.R & REACTIVE_DISPOSED) && flattenIfIterable(r3, e)) return;
            asyncWrite(r3);
          } catch (e2) {
            handleError(e2);
          }
          settleAutodispose();
        }
      }, (e2) => {
        if (f2) {
          o2 = e2;
          r2 = true;
        } else {
          handleError(e2);
          settleAutodispose();
        }
      });
      f2 = false;
      if (r2) {
        handleError(o2);
        throw o2;
      } else if (!n2) {
        if (e.ce) return e.Ie;
        throw new NotReadyError(context);
      } else if (!flattenIfIterable(u)) {
        e.ce = false;
      }
    }
    if (o) flattenIfIterable(t2);
    if (s !== null) {
      if (!s) {
        if (e.ce) return e.Ie;
        throw new NotReadyError(context);
      }
      e.ce = false;
    }
    return u;
  }
  function clearStatus(e, t2 = false) {
    if (e.T?.fe) clearPendingSources(e);
    if (e.T?.ue) {
      if (e.T !== null) e.T.ue = false;
    }
    e.m = t2 ? 0 : e.m & STATUS_UNINITIALIZED;
    if (e.T?.F) setPendingError(e);
    const n = statusNotifierOf(e);
    if (n) n.call(e);
  }
  function landStatus(e, t2 = false) {
    const n = e.T?.fe;
    if (n && (n.delete(e), n.size)) {
      e.T.ue = false;
      if (t2) e.m = STATUS_PENDING;
      setPendingError(e, n.values().next().value);
    } else clearStatus(e, t2);
  }
  function notifyStatus(e, t2, n, r) {
    if (t2 === STATUS_ERROR && !(n instanceof StatusError) && !(n instanceof NotReadyError)) n = new StatusError(e, n);
    const o = t2 === STATUS_PENDING && n instanceof NotReadyError ? n.source : void 0;
    {
      if (t2 === STATUS_PENDING && o) {
        if (e.m & STATUS_ERROR) e.T.de = e.T.F;
        addPendingSource(e, o);
        e.m = STATUS_PENDING | e.m & STATUS_UNINITIALIZED;
        setPendingError(e, o, n);
      } else {
        clearPendingSources(e);
        e.m = t2 | (t2 !== STATUS_ERROR ? e.m & STATUS_UNINITIALIZED : 0);
        ext(e).F = n;
      }
    }
    if (t2 === STATUS_PENDING && e.l & CONFIG_GUESS) {
      if (passLane === null) joinFuture(null);
      return;
    }
    const f = statusNotifierOf(e);
    if (f) {
      {
        f.call(e);
      }
      return;
    }
    propagateStatus(e, t2, n);
  }
  function propagateStatus(e, t2, n, r) {
    const o = t2 === STATUS_PENDING && n instanceof NotReadyError ? n.source : void 0;
    forEachDependent(e, (r2, f) => {
      r2.ie = clock;
      if (r2.l & (CONFIG_REDERIVE | CONFIG_VERDICT) || t2 === STATUS_PENDING && f.Te !== r2.Ne) {
        enqueueSub(r2);
        schedule();
        return;
      }
      if (t2 === STATUS_PENDING && o && r2.T?.fe?.has(o)) {
        if (passLane === null && (r2.l & (CONFIG_HELD | CONFIG_OVERRIDE)) === CONFIG_HELD && globalQueue.q && !freshHook?.(r2, o.ie === clock)) joinFuture(txOf(r2));
        return;
      }
      if (t2 === STATUS_PENDING && o || t2 !== STATUS_PENDING && (r2.T?.F !== n || r2.T?.fe)) {
        {
          if (laneStageHook && passLane !== null) laneStageHook(r2, passLane, true, e);
          else {
            queuePendingNode(r2);
            if (t2 === STATUS_PENDING && (r2.l & (CONFIG_HELD | CONFIG_OVERRIDE)) === CONFIG_HELD && !r2.i) joinFuture(txOf(r2));
          }
        }
        notifyStatus(r2, t2, n);
      }
    });
  }

  // node_modules/.pnpm/@solidjs+signals@2.0.0-rc.17/node_modules/@solidjs/signals/dist/prod/core/core.js
  GlobalQueue._e = (e) => {
    if (e.i === EFFECT_TRACKED) {
      deleteFromHeap(e, dirtyQueue);
      e.Ae = true;
      globalQueue.enqueue(EFFECT_USER, e.M);
    } else if (e.R & REACTIVE_ZOMBIE && !(e.R & REACTIVE_LANE_DIRTY)) deferZombie(e);
    else if (laneStageHook && e.R & REACTIVE_ZOMBIE && laneFrameMember(e)) {
      deleteFromHeap(e, dirtyQueue);
      e.R &= -65540;
    } else recompute(e);
  };
  function laneFrameMember(e) {
    let t2 = e._parent;
    while (t2.R & REACTIVE_ZOMBIE) t2 = t2._parent;
    return (t2.l & (CONFIG_OVERRIDE | CONFIG_GUESS)) === CONFIG_OVERRIDE || t2.i && t2.T?.Re?.v;
  }
  GlobalQueue.Ce = disposeChildren;
  var tracking = false;
  var context = null;
  var snapshotCaptureActive = false;
  var laneRoot;
  var snapshotSources = null;
  function ownerInSnapshotScope(e) {
    while (e) {
      if (e.Oe) return true;
      e = e._parent;
    }
    return false;
  }
  function recompute(e, t2 = false) {
    bumpNotifyEpoch();
    const n = e.i;
    let o;
    const E = passLane;
    const s = laneRoot;
    const i = laneStageHook && (laneRoot = t2 && (E || s));
    setPassLane(!laneStageHook ? null : (e.R & REACTIVE_LANE_DIRTY || (e.l & (CONFIG_OVERRIDE | CONFIG_GUESS)) === CONFIG_OVERRIDE) && e.T?.Re?.v ? e.T.Re : (
      // Loose `!=`: false with no creator lane (undefined == null).
      t2 && E?._parent.he != E && creatorPass(context)?.R & REACTIVE_RECOMPUTING_DEPS ? E : null
    ));
    if (!t2) {
      deleteFromHeap(e, dirtyQueue);
      if (e.T !== null) {
        e.T.Se = null;
        releaseFlightTeardown(e);
      }
      if (e.m & STATUS_PENDING) e.l &= ~CONFIG_VERDICT;
      if (e.l & CONFIG_HELD) {
        const t3 = txOf(e);
        if (t3.v || e.l & CONFIG_LAGS) ;
        else if (n !== EFFECT_RENDER && !(e.l & CONFIG_VERDICT)) joinPassTx(t3, freshHook?.(e, e.m & STATUS_UNINITIALIZED));
        else if (t3 !== flushTransaction && !(e.m & STATUS_UNINITIALIZED || e.l & CONFIG_HELD_FRAME)) {
          e.se = NOT_PENDING;
          if (n === EFFECT_RENDER) e.l &= ~CONFIG_HELD;
        }
      }
      if (n === EFFECT_TRACKED || e.l & CONFIG_STAGED) {
        const t3 = deferredReleases;
        deferReleases(o = []);
        disposeChildren(e);
        deferReleases(t3);
      } else if (e.De !== null || e.He !== null) parkChildren(e);
      if (e.l & CONFIG_STAGED_ERROR) {
        e.se = NOT_PENDING;
        e.l ^= CONFIG_STAGED_ERROR;
      }
    }
    const l = !!(e.m & STATUS_UNINITIALIZED);
    const r = e.m & STATUS_ERROR ? e.T?.F : void 0;
    const u = (e.m & STATUS_PENDING) !== 0;
    const _ = u ? e.T?.fe : void 0;
    const a = e.T?.fe?.has(e);
    let I = false;
    const N = (e.R & REACTIVE_REASK) !== 0 && !(e.m & STATUS_PENDING && e.T !== null && !e.T.Ve);
    const f = context;
    context = e;
    e._ = null;
    e.Ne++;
    e.R = REACTIVE_RECOMPUTING_DEPS | e.R & (REACTIVE_ZOMBIE | REACTIVE_REASK);
    e.ie = clock;
    let T = e.se !== NOT_PENDING ? e.se : e.l & CONFIG_OVERRIDE && e.T.v !== NOT_PENDING ? e.T.v : e.Ie;
    let S = e.Fe;
    let c = false;
    let A = false;
    let R = tracking;
    tracking = true;
    try {
      if (e.l & CONFIG_SYNC) {
        T = e.X(T);
        if (e.T !== null) e.T.Se = null;
        e.ce = false;
      } else {
        const t3 = e.T?.Se;
        const n2 = e.X(T);
        const o2 = typeof n2 === "object" && n2 !== null;
        const E2 = e.T?.Se !== t3;
        T = E2 || !o2 ? n2 : handleAsync(e, n2);
        if (!E2 && !o2) {
          if (e.T !== null) e.T.Se = null;
          e.ce = false;
        }
      }
      if (e.m !== 0 || e.T !== null) clearStatus(e, t2);
    } catch (t3) {
      const n2 = t3 instanceof NotReadyError;
      if (n2 && e.ce) {
        parkLoadingWindow(e, t3);
      } else {
        if (n2 || !(I = stageError(e, t3, true))) {
          if (n2) ext(e).ue = true;
          notifyStatus(e, n2 ? STATUS_PENDING : STATUS_ERROR, t3);
          if (n2 && a && !e.T?.Se) settlePendingSource(e);
        }
        if ((n2 || I) && _) {
          for (const t4 of _) if (t4 !== e && !e.T?.fe?.has(t4)) settlePendingSource(e, t4);
        }
      }
    } finally {
      tracking = R;
      if (laneStageHook) laneRoot = s;
      c = (e.R & REACTIVE_MISSED_WAKE) !== 0;
      A = !!(e.R & (i && (E || n || !(e.m & STATUS_PENDING)) ? REACTIVE_JOINED | REACTIVE_STAGED_READ : REACTIVE_JOINED));
      e.l = e.l & ~CONFIG_VERDICT | e.R & REACTIVE_PROBED;
      e.R = e.R & (REACTIVE_ZOMBIE | REACTIVE_DISPOSED | REACTIVE_FRAME_READ | REACTIVE_STAGED_READ | REACTIVE_LANE_READ | REACTIVE_SCREEN_READ | REACTIVE_PROBE_UNANSWERED) | (t2 ? e.R & REACTIVE_SNAPSHOT_STALE : 0);
      context = f;
      if (o) for (const e2 of o) releaseIfSettledUnobserved(e2);
      if (e.T !== null) e.T.Ve = N && (e.m & STATUS_PENDING) !== 0;
    }
    if (e.R & REACTIVE_DISPOSED) {
      clearDeps(e);
      if (e.T !== null) e.T.Se = null;
      setPassLane(E);
      return;
    }
    let C = !laneStageHook ? null : passLane;
    const d = !!e.T?.F || I;
    if (laneStageHook && C !== null) {
      if (laneStageHook(e, C, t2, d)) setPassLane(C);
      else C = null;
    }
    if (!d) {
      const o2 = e.se !== NOT_PENDING ? e.se : e.l & CONFIG_OVERRIDE && e.T.v !== NOT_PENDING ? e.T.v : e.Ie;
      const E2 = !n && l || !!e.T?.de || !isSame(e, o2, T);
      if (n && E2 && // A stale reader of a lane (REACTIVE_SCREEN_READ) that computed what it
      // last applied owes no run (lanes.ts).
      !(e.R & REACTIVE_SCREEN_READ && !e.Ae && Object.is(e.ge, T)) && // A re-staging pass owes no run: the commit replays the effect (#3802).
      e.se === NOT_PENDING) {
        e.Ae = !e.T?.F;
        if (!t2 || C !== null) globalQueue.enqueue(n, e.W ??= GlobalQueue.K.bind(null, e), C);
      }
      if (!t2 && C === null && e.l & CONFIG_GUESS && laneOutcomeHook(e, T, d)) ;
      else if (d) ;
      else if (E2) {
        if (C !== null) {
          if (n && e.se === NOT_PENDING) e.Ie = T;
          else if (n || C.ke) e.se = T;
          else e.T.v = T;
        } else if (t2 ? !((flushTransaction !== null || passTx !== null) && (A || creatorPass(f)?.R & REACTIVE_JOINED)) : n && e.se === NOT_PENDING) {
          e.Ie = T;
        } else {
          e.se = T;
          if (t2) {
            holdNode(e, flushTransaction ?? passTx);
            e.m |= STATUS_UNINITIALIZED;
            freshHook?.(e, !i && flushTransaction);
          }
        }
        if (e.o !== null) {
          insertSubs(e);
          if (C !== null) laneDirty(e, C);
        }
      } else if (e.Fe != S) {
        for (let t3 = e.o; t3 !== null; t3 = t3.t) insertIntoHeapHeight(t3.u, dirtyQueue);
      }
      if (!E2 && !e.T?.F) {
        if (r !== void 0) settleErroredDependents(e, r);
        if (_) {
          for (const t3 of _) if (t3 !== e) settlePendingSource(e, t3);
        }
      }
      if (a && !(e.m & (STATUS_PENDING | STATUS_UNINITIALIZED))) settlePendingSource(e);
    }
    if (t2 && C === null && A && e.m & STATUS_PENDING) {
      const t3 = flushTransaction ?? passTx;
      if (t3 && !t3.v && freshHook?.(e) !== CAUGHT_REARMED) holdNode(e, t3);
    }
    if (!e.T?.F && e.se === NOT_PENDING && !(n && e.Ae)) {
      if (t2 || n === EFFECT_TRACKED) trimStaleDeps(e);
      else heldTrim(e);
    }
    if (e.se !== NOT_PENDING || e.T !== null && (e.T.Ue !== null || e.T.me !== null) || (e.m & (STATUS_PENDING | STATUS_UNINITIALIZED)) !== 0 && (!t2 || (e.m & STATUS_PENDING) !== 0)) {
      e.l |= CONFIG_STAGED;
      if (C === null) queuePendingNode(e);
    }
    setPassLane(E);
    if (c) {
      enqueueSub(e);
      schedule();
    }
  }
  function updateIfNecessary(e) {
    if (e.R & (REACTIVE_RECOMPUTING_DEPS | REACTIVE_DISPOSED)) return;
    if (e.R & REACTIVE_CHECK) {
      for (let t2 = e.C; t2; t2 = t2.G) {
        const n = t2.S;
        if (n.X) {
          updateIfNecessary(n);
        }
        if (e.R & REACTIVE_DIRTY) {
          break;
        }
      }
    }
    if (e.R & REACTIVE_DIRTY || e.T?.F && e.ie < clock && !e.T?.Se) {
      recompute(e);
    }
    e.R = e.R & (REACTIVE_SNAPSHOT_STALE | REACTIVE_IN_HEAP | REACTIVE_IN_HEAP_HEIGHT | REACTIVE_ZOMBIE | REACTIVE_DISPOSED | REACTIVE_MANUAL_WRITE | REACTIVE_FRAME_READ | REACTIVE_STAGED_READ | REACTIVE_LANE_READ | REACTIVE_SCREEN_READ | REACTIVE_PROBE_UNANSWERED);
  }
  function computed(e, t2) {
    const n = t2?.transparent ?? false;
    const o = t2 !== null && typeof t2 === "object" && "loadingValue" in t2;
    const E = {
      id: inheritId(t2, n, context),
      l: (n ? CONFIG_TRANSPARENT : 0) | (t2?.ownedWrite ? CONFIG_OWNED_WRITE : 0) | (!context || t2?.lazy ? CONFIG_AUTO_DISPOSE : 0) | (t2?.sync ? CONFIG_SYNC : 0) | (t2?.le ? CONFIG_NO_SNAPSHOT : 0) | (t2?.Le ?? 0) | (snapshotCaptureActive && ownerInSnapshotScope(context) ? CONFIG_IN_SNAPSHOT_SCOPE : 0),
      xe: t2?.equals ?? isEqual,
      He: null,
      A: context?.A ?? defaultContext,
      ye: 0,
      X: e,
      Ie: o ? t2.loadingValue : void 0,
      Fe: 0,
      be: void 0,
      We: null,
      C: null,
      _: null,
      Ne: 0,
      o: null,
      ve: null,
      _parent: context,
      pe: null,
      Ze: null,
      De: null,
      R: t2?.lazy ? REACTIVE_LAZY : REACTIVE_NONE,
      // A loadingValue node is born committed: commit #0 is already in _value.
      m: o ? 0 : STATUS_UNINITIALIZED,
      ie: clock,
      se: NOT_PENDING,
      Me: -1,
      ce: o,
      // Cold machinery (async slots) lives one hop away in the lazily-allocated extension — the core literal MUST
      // stay under V8's in-object boundary (§12: past ~39 fields every
      // allocation spills to a backing store and creation cost ~4x's).
      T: null
    };
    if (t2?.unobserved) ext(E).we = t2.unobserved;
    setupComputedNode(E, t2);
    return E;
  }
  function ext(e) {
    return e.T ??= {
      Se: null,
      ae: null,
      F: void 0,
      ue: void 0,
      fe: void 0,
      we: void 0,
      Pe: void 0,
      Ue: null,
      me: null,
      Re: null,
      Ve: false,
      je: NOT_PENDING,
      Be: -1,
      Ee: 0,
      v: NOT_PENDING,
      I: 0,
      de: void 0
    };
  }
  function joinPass(e, t2) {
    e.R |= REACTIVE_JOINED;
    if (t2.m & STATUS_UNINITIALIZED || e.i !== EFFECT_RENDER) joinPassTx(txOf(t2), !e.i && e.m & STATUS_UNINITIALIZED || freshHook?.(e));
  }
  function frameRead(e, t2) {
    const n = observeFlightHook && e.l & CONFIG_VERDICT;
    const o = laneStageHook && passLane !== null;
    if ((!o || e.m & STATUS_UNINITIALIZED && t2.m & STATUS_PENDING) && !n && (e.i !== EFFECT_RENDER || t2.l & CONFIG_INPUTS_PUBLISHED)) return false;
    const E = txOf(t2);
    if (E === flushTransaction && !o && !n || bornInto(e, E)) return false;
    staleReader(e, E);
    return true;
  }
  function bornInto(e, t2) {
    return (e.m & STATUS_UNINITIALIZED || e.l & CONFIG_HELD_FRAME) && e.l & CONFIG_HELD && txOf(e) === t2;
  }
  function creatorPass(e) {
    return e !== null && e.Qe ? e.Ye : e;
  }
  function parkChildren(e) {
    markDisposal(e);
    const t2 = ext(e);
    t2.me = e.He;
    t2.Ue = e.De;
    e.He = null;
    e.De = null;
    e.ye = 0;
  }
  function createEffectNode(e, t2, n, o, E) {
    const s = E?.transparent ?? false;
    const i = {
      id: inheritId(E, s, context),
      l: (s ? CONFIG_TRANSPARENT : 0) | (E?.ownedWrite ? CONFIG_OWNED_WRITE : 0) | (E?.sync ? CONFIG_SYNC : 0) | (E?.Le ?? 0) | (snapshotCaptureActive && ownerInSnapshotScope(context) ? CONFIG_IN_SNAPSHOT_SCOPE : 0),
      xe: false,
      He: null,
      A: context?.A ?? defaultContext,
      ye: 0,
      X: e,
      Ie: void 0,
      Fe: 0,
      be: void 0,
      We: null,
      C: null,
      _: null,
      Ne: 0,
      o: null,
      ve: null,
      _parent: context,
      pe: null,
      Ze: null,
      De: null,
      R: REACTIVE_LAZY,
      m: STATUS_UNINITIALIZED,
      ie: clock,
      se: NOT_PENDING,
      Me: -1,
      ce: false,
      Ae: false,
      ge: void 0,
      Ke: t2,
      qe: n,
      Je: void 0,
      i: o,
      T: null
    };
    if (E?.unobserved) ext(i).we = E.unobserved;
    setupComputedNode(i, lazyOptions);
    return i;
  }
  var effectStatusNotify = null;
  function setEffectStatusNotify(e) {
    effectStatusNotify = e;
  }
  function statusNotifierOf(e) {
    return e.i ? effectStatusNotify ?? void 0 : void 0;
  }
  var lazyOptions = {
    lazy: true
  };
  function setupComputedNode(e, t2) {
    e.We = e;
    const n = context?.Qe ? context.Ye : context;
    if (context) linkChild(context, e);
    if (n) e.Fe = n.Fe + 1;
    wireExternalSourceHook?.(e);
    !t2?.lazy && recompute(e, true);
    if (snapshotCaptureActive && !t2?.lazy) {
      if (!(e.m & STATUS_PENDING) && !(e.l & CONFIG_NO_SNAPSHOT)) {
        ext(e).Pe = e.Ie === void 0 ? NO_SNAPSHOT : e.Ie;
        e.l |= CONFIG_HAS_SNAPSHOT;
        snapshotSources.add(e);
      }
    }
  }
  function signal(e, t2) {
    const n = {
      xe: t2?.equals ?? isEqual,
      l: (t2?.ownedWrite ? CONFIG_OWNED_WRITE : 0) | (t2?.le ? CONFIG_NO_SNAPSHOT : 0),
      Ie: e,
      o: null,
      ve: null,
      ie: clock,
      se: NOT_PENDING,
      // Signal-literal diet (§12e): NO _time/_fn/_statusFlags slots. Stores
      // materialize one signal per touched leaf, so signal bytes are store
      // bytes. _time is write-only on signals (every read site is computed-
      // typed error-retry gating); _fn/_statusFlags read falsy-identically as
      // missing properties on the shared paths (undefined masks to 0).
      Me: -1,
      T: null
    };
    if (t2?.unobserved) ext(n).we = t2.unobserved;
    if (snapshotCaptureActive && !(n.l & CONFIG_NO_SNAPSHOT)) {
      ext(n).Pe = e === void 0 ? NO_SNAPSHOT : e;
      n.l |= CONFIG_HAS_SNAPSHOT;
      snapshotSources.add(n);
    }
    return n;
  }
  function isEqual(e, t2) {
    return e === t2;
  }
  var verdict = null;
  function untrack(e, t2) {
    const n = tracking;
    tracking = false;
    try {
      if (externalUntrackHook) return externalUntrackHook(e);
      return e();
    } finally {
      tracking = n;
    }
  }
  var spectating = false;
  function prepareComputed(e, t2) {
    if (e.R & REACTIVE_LAZY) {
      e.R &= ~REACTIVE_LAZY;
      recompute(e, true);
    } else if (e.R & REACTIVE_DISPOSED) {
      if (e.l & CONFIG_AUTO_DISPOSE) {
        const t3 = e._parent;
        if (t3 !== null) {
          if (t3.R & REACTIVE_DISPOSED) {
            e.l &= ~CONFIG_AUTO_DISPOSE;
            return;
          }
          if (!(e.R & REACTIVE_ZOMBIE)) linkChild(t3, e);
        }
        recompute(e, true);
      }
    } else ;
  }
  function readerSeesCommitted(e, t2) {
    return !t2 || e.se === NOT_PENDING || t2.l & CONFIG_CHILDREN_FORBIDDEN;
  }
  var unflushedStaged = false;
  function markUnflushedStaged() {
    unflushedStaged = true;
  }
  function unflushedValue(e) {
    if (globalQueue.q || e.se === NOT_PENDING || e.l & CONFIG_PROMOTED) return NOT_PENDING;
    if (e.l & CONFIG_HELD) return e.T.Be === clock ? e.T.je : NOT_PENDING;
    return e.Ie;
  }
  function stashFlushed(e) {
    const t2 = e.T;
    if (t2.Be !== clock) {
      t2.je = e.se;
      t2.Be = clock;
      unflushedStaged = true;
    }
  }
  var promotedWrites = [];
  function markLateLinker(e) {
    e.R |= REACTIVE_MISSED_WAKE;
    return true;
  }
  function resyncUnflushedCompanions() {
    unflushedStaged = false;
    if (promotedWrites.length !== 0) {
      for (const e of promotedWrites) e.l &= ~CONFIG_PROMOTED;
      promotedWrites.length = 0;
    }
  }
  function pullComputed(e, t2) {
    if (e.Fe >= dirtyQueue.Ge) {
      markNode(t2);
      markHeap(dirtyQueue);
      updateIfNecessary(e);
    } else if (t2.l & CONFIG_FRESH_READ) updateIfNecessary(e);
    const n = e.Fe;
    if (n >= t2.Fe && e._parent !== t2) t2.Fe = n + 1;
  }
  function read(e) {
    let t2 = context;
    if (t2?.Qe) t2 = t2.Ye;
    const n = e;
    const o = e;
    if (typeof n.X === "function") prepareComputed(e);
    if (verdict !== null) return verdict(e, t2);
    if (!n.X && e.T?.Pe === void 0 && !snapshotCaptureActive && (!unflushedStaged || e.se === NOT_PENDING) && // A28, see readNodeFast
    !(e.l & CONFIG_OVERRIDE) && // lanes: the slow path's one arm
    true) {
      if (t2) {
        if (tracking) link(e, t2);
        if (e.l & CONFIG_HELD && !(t2.l & CONFIG_CHILDREN_FORBIDDEN)) {
          if (frameRead(t2, e)) return e.Ie;
          joinPass(t2, e);
        }
      }
      if (!t2 || e.se === NOT_PENDING || t2.l & CONFIG_CHILDREN_FORBIDDEN) return e.Ie;
      if ((observeFlightHook && t2.l & CONFIG_VERDICT || laneStageHook && passLane?.folded) && stagedScreen(t2)) return e.Ie;
      stagedRead(t2);
      return e.se;
    }
    let E = false;
    if (t2) {
      if (tracking) {
        link(e, t2);
        if (o.X) {
          pullComputed(o, t2);
          if (!t2.i) t2.R |= o.l & CONFIG_VERDICT;
        }
      }
      if (e.l & CONFIG_HELD && !(e.l & CONFIG_OVERRIDE) && !(t2.l & CONFIG_CHILDREN_FORBIDDEN)) {
        if (o.m & STATUS_UNINITIALIZED || !frameRead(t2, e)) joinPass(t2, e);
        else E = true;
      }
    }
    if (laneReadHook && e.l & CONFIG_OVERRIDE) {
      const n2 = laneReadHook(t2, e);
      if (n2 !== NOT_PENDING) return n2;
    }
    if (o.m & STATUS_PENDING && !E && !(e.l & CONFIG_OVERRIDE) && !(o.m & STATUS_UNINITIALIZED)) {
      if (laneStageHook && passLane !== null && !(t2?.m & STATUS_UNINITIALIZED)) E = true;
      else if (observeFlightHook && t2 !== null && t2.l & CONFIG_VERDICT) E = observeFlightHook(t2, o);
    }
    if (o.m & STATUS_PENDING && !E) {
      if (t2) {
        if (!(t2.l & CONFIG_CHILDREN_FORBIDDEN) || o.m & STATUS_UNINITIALIZED) {
          if (!tracking && !spectating && e !== t2) link(e, t2);
          throw o.T?.F;
        }
      } else if (o.m & STATUS_UNINITIALIZED) {
        throw o.T?.F;
      }
    }
    if (o.X && o.m & STATUS_ERROR) {
      if (tracking && o.ie < clock) {
        recompute(o);
        return read(e);
      } else throw o.T?.F;
    }
    if (snapshotCaptureActive && t2 && t2.l & CONFIG_IN_SNAPSHOT_SCOPE) {
      const n2 = e.T?.Pe;
      if (n2 !== void 0) {
        const o2 = n2 === NO_SNAPSHOT ? void 0 : n2;
        const E2 = e.se !== NOT_PENDING ? e.se : e.Ie;
        if (E2 !== o2) t2.R |= REACTIVE_SNAPSHOT_STALE;
        return o2;
      }
    }
    if (E) return e.Ie;
    if (!t2 && e.T?.de) throw e.T.de;
    const s = serve(e, t2);
    if (!t2 && typeof n.X === "function" && e.l & CONFIG_AUTO_DISPOSE && !(o.m & STATUS_PENDING) && !e.o) {
      dormantNodes.add(e);
      schedule();
    }
    return s;
  }
  function serve(e, t2) {
    if (e.se !== NOT_PENDING && e.m & STATUS_UNINITIALIZED && (!t2 || spectating || t2.l & CONFIG_CHILDREN_FORBIDDEN)) throw new NotReadyError(null);
    const n = t2 && unflushedStaged ? unflushedValue(e) : NOT_PENDING;
    if (n !== NOT_PENDING) {
      markLateLinker(t2);
      return n;
    }
    if (readerSeesCommitted(e, t2)) return e.Ie;
    if (observeFlightHook && t2.l & CONFIG_VERDICT && !(e.m & STATUS_UNINITIALIZED) && stagedScreen(t2)) return e.Ie;
    stagedRead(t2, e);
    if (e.l & CONFIG_STAGED_ERROR) throw e.se;
    return e.se;
  }
  function stagedRead(e, t2) {
    if (!(t2?.m & STATUS_UNINITIALIZED)) {
      e.R |= REACTIVE_STAGED_READ;
      if (laneSeamsHook && passLane !== null) stagedReaders.push(e);
    }
  }
  function stagedScreen(e) {
    if (flushTransaction === null || bornInto(e, flushTransaction)) return false;
    staleReader(e, flushTransaction);
    return true;
  }
  function notePromotedWrite(e) {
    if (globalQueue.q || e.l & CONFIG_PROMOTED) return;
    e.l |= CONFIG_PROMOTED;
    promotedWrites.push(e);
  }
  function captureWriteSnapshot(e, t2) {
    if (e.l & CONFIG_NO_SNAPSHOT || e.X !== void 0 || e.T?.Pe !== void 0) return;
    ext(e).Pe = t2 === void 0 ? NO_SNAPSHOT : t2;
    e.l |= CONFIG_HAS_SNAPSHOT;
    snapshotSources.add(e);
  }
  var writeHook = null;
  function isSame(e, t2, n) {
    if (e.xe === isEqual) return t2 === n;
    try {
      return e.xe && e.xe(t2, n);
    } catch (t3) {
      if (!(t3 instanceof NotReadyError)) return notifyStatus(e, STATUS_ERROR, t3), true;
    }
  }
  function setSignal(e, t2) {
    if (writeHook !== null) writeHook(e);
    const n = e.se === NOT_PENDING ? e.Ie : e.se;
    if (typeof t2 === "function") t2 = t2(n);
    if (laneWriteHook && e.l & CONFIG_OVERRIDE) return laneWriteHook(e, t2);
    if (e.l & CONFIG_HELD) joinFuture(txOf(e));
    const o = !!(e.m & STATUS_UNINITIALIZED) || !!e.T?.de || !isSame(e, n, t2);
    if (!o) return t2;
    if (snapshotCaptureActive) captureWriteSnapshot(e, n);
    const E = e.se !== NOT_PENDING;
    if (!E) queuePendingNode(e);
    else if (e.l & CONFIG_HELD && !globalQueue.q) stashFlushed(e);
    e.se = t2;
    if (context !== null) notePromotedWrite(e);
    if (e.X !== void 0) e.ie = clock;
    if (E && e.Me === notifyEpoch) return t2;
    insertSubs(e);
    schedule();
    return t2;
  }
  function setMemo(e, t2) {
    const n = e.l & CONFIG_HELD && !(e.R & REACTIVE_MANUAL_WRITE) && txOf(e) !== flushTransaction;
    if (n && typeof t2 === "function") {
      const n2 = t2(e.Ie);
      t2 = () => n2;
    }
    if (e.m & STATUS_ERROR) {
      e.T.de = e.T.F;
      notifyStatus(e, STATUS_NONE, void 0);
    }
    t2 = setSignal(e, t2);
    e.l &= ~CONFIG_STAGED_ERROR;
    if (n) {
      e.R = e.R & ~REACTIVE_CHECK | REACTIVE_DIRTY;
      insertIntoHeap(e, dirtyQueue);
      schedule();
    } else if (e.se !== NOT_PENDING) e.R |= REACTIVE_MANUAL_WRITE;
    return t2;
  }
  function runWithOwner(e, t2) {
    const n = context;
    const o = tracking;
    context = e;
    tracking = false;
    try {
      return t2();
    } finally {
      context = n;
      tracking = o;
    }
  }
  function staleValues(e, t2 = true) {
    try {
      return e();
    } finally {
    }
  }

  // node_modules/.pnpm/@solidjs+signals@2.0.0-rc.17/node_modules/@solidjs/signals/dist/prod/core/identities.js
  var signalHook = null;

  // node_modules/.pnpm/@solidjs+signals@2.0.0-rc.17/node_modules/@solidjs/signals/dist/prod/core/context.js
  function setContext(e, t2, r = getOwner()) {
    if (!r) {
      throw new NoOwnerError();
    }
    r.A = {
      ...r.A,
      [e.id]: t2 === void 0 ? e.defaultValue : t2
    };
  }

  // node_modules/.pnpm/@solidjs+signals@2.0.0-rc.17/node_modules/@solidjs/signals/dist/prod/core/effect.js
  function effect(t2, e, E, u) {
    const f = !!u?.user;
    const r = createEffectNode(t2, e, E, f ? EFFECT_USER : EFFECT_RENDER, u);
    recompute(r, true);
    !u?.defer && r.se === NOT_PENDING && !r.T?.Re?.v && (r.i === EFFECT_USER || u?.schedule ? globalQueue.enqueue(r.i, runEffect.bind(null, r)) : runEffect(r, r.i));
  }
  function notifyEffectStatus(t2, e) {
    const E = t2 !== void 0 ? t2 : this.m;
    const u = e !== void 0 ? e : this.T?.F;
    if (E & STATUS_ERROR) {
      globalQueue.notify(this, STATUS_PENDING, 0);
      if (this.i === EFFECT_USER) {
        if (this.m & STATUS_ERROR) {
          this.Ae = true;
          globalQueue.enqueue(this.i, this.W ??= runEffect.bind(null, this));
        }
        return;
      }
      if (!globalQueue.notify(this, STATUS_ERROR, STATUS_ERROR)) {
        haltReactivity(unwrapStatusError(u));
        throw u;
      }
    } else if (this.i === EFFECT_RENDER) {
      globalQueue.notify(this, STATUS_PENDING | STATUS_ERROR, E, u);
    }
  }
  function runEffect(t2, e) {
    if (!t2.Ae || t2.R & REACTIVE_DISPOSED) return;
    if (heldRunHook?.(t2)) return;
    if (t2.m & STATUS_ERROR && t2.i === EFFECT_USER) {
      const e2 = unwrapStatusError(t2.T?.F);
      t2.ge = t2.Ie;
      t2.Ae = false;
      try {
        t2.qe ? t2.qe(e2, () => {
          const e3 = t2.Je;
          t2.Je = void 0;
          e3?.();
        }) : console.error(e2);
      } catch (e3) {
        if (!globalQueue.notify(t2, STATUS_ERROR, STATUS_ERROR)) {
          haltReactivity(e3);
          throw e3;
        }
      }
      return;
    }
    const E = t2.T?.F == null;
    const u = t2.Je;
    t2.Je = void 0;
    try {
      u?.();
      const e2 = t2.Ke(t2.Ie, t2.ge);
      if (false) ;
      t2.Je = e2;
    } catch (e2) {
      ext(t2).F = new StatusError(t2, e2);
      t2.m |= STATUS_ERROR;
      if (!globalQueue.notify(t2, STATUS_ERROR, STATUS_ERROR)) {
        haltReactivity(e2);
        throw e2;
      }
    } finally {
      t2.ge = t2.Ie;
      t2.Ae = false;
      if (E) trimStaleDeps(t2);
    }
  }
  GlobalQueue.K = runEffect;
  function trackedEffect(t2, e) {
    const run = () => {
      if (!E.Ae || E.R & REACTIVE_DISPOSED) return;
      if (heldRunHook?.(E)) return;
      try {
        E.Ae = false;
        recompute(E);
      } finally {
      }
    };
    const E = computed(() => {
      const e2 = E.Je;
      E.Je = void 0;
      e2?.();
      const u = staleValues(t2);
      E.Je = u;
    }, {
      ...e,
      lazy: true
    });
    E.Je = void 0;
    E.l = E.l & ~CONFIG_AUTO_DISPOSE | CONFIG_CHILDREN_FORBIDDEN;
    E.Ae = true;
    E.i = EFFECT_TRACKED;
    E.M = run;
    enqueueSub(E);
    schedule();
  }
  setEffectStatusNotify(notifyEffectStatus);

  // node_modules/.pnpm/@solidjs+signals@2.0.0-rc.17/node_modules/@solidjs/signals/dist/prod/core/error-hooks.js
  var ambientHook;
  var reported = /* @__PURE__ */ new WeakSet();
  function hookFor(e) {
    for (let n = e; n; n = n._parent) {
      const e2 = n[ROOT_ERROR_HOOK];
      if (e2 !== void 0) return e2;
    }
    return ambientHook;
  }
  function labels(e) {
    const n = [];
    for (let o = e; o; o = o._parent) {
      const e2 = o._name;
      if (typeof e2 === "string" && e2.length) n.push(e2);
    }
    return n.length ? n.reverse() : void 0;
  }
  function reportClientError(e, n, o) {
    const r = e !== null && (typeof e === "object" || typeof e === "function");
    if (r) {
      if (reported.has(e)) return;
      reported.add(e);
    }
    const t2 = hookFor(n);
    if (t2 === void 0) return;
    const i = {};
    const f = labels(n);
    const c = labels(o) ?? f;
    if (c !== void 0) i.ownerPath = c;
    if (f !== void 0) i.boundaryPath = f;
    try {
      t2(e, i);
    } catch (e2) {
      console.error(e2);
    }
  }

  // node_modules/.pnpm/@solidjs+signals@2.0.0-rc.17/node_modules/@solidjs/signals/dist/prod/store/types.js
  var $TRACK = Symbol(0);
  var $TARGET = Symbol(0);
  var $PROXY = Symbol(0);
  var $RECORD = Symbol(0);
  var OBJECT_PROTO = Object.prototype;
  function ownEnumerableKeys(e) {
    return Reflect.ownKeys(e).filter((r) => Object.prototype.propertyIsEnumerable.call(e, r));
  }

  // node_modules/.pnpm/@solidjs+signals@2.0.0-rc.17/node_modules/@solidjs/signals/dist/prod/signals.js
  function onCleanup(e) {
    return cleanup(e);
  }
  function accessor(e) {
    const t2 = read.bind(null, e);
    t2[$REFRESH] = e;
    return t2;
  }
  function createSignal(e, t2) {
    if (typeof e === "function") {
      const r2 = computed(e, t2);
      r2.l &= ~CONFIG_AUTO_DISPOSE;
      return [accessor(r2), setMemo.bind(null, r2)];
    }
    const r = signal(e, t2);
    if (signalHook !== null) signalHook(r);
    return [accessor(r), setSignal.bind(null, r)];
  }
  function createMemo(e, t2) {
    return accessor(computed(e, t2));
  }
  function createRenderEffect(e, t2, r) {
    effect(e, t2, void 0, r);
  }
  function createTrackedEffect(e, t2) {
    trackedEffect(e, t2);
  }

  // node_modules/.pnpm/@solidjs+signals@2.0.0-rc.17/node_modules/@solidjs/signals/dist/prod/map.js
  function mapArray(t2, e, i) {
    const s = typeof i?.keyed === "function" ? i.keyed : void 0;
    const n = e.length > 1;
    const r = e;
    const o = {
      L: createOwner(),
      it: 0,
      st: t2,
      ot: [],
      ft: r,
      ht: [],
      tn: [],
      ze: s,
      ct: s || i?.keyed === false ? [] : void 0,
      lt: n && i?.keyed !== false ? [] : void 0,
      ut: i?.keyed === false,
      H: i?.fallback
    };
    const f = computed(updateKeyedMap.bind(o), void 0);
    f.l &= ~CONFIG_AUTO_DISPOSE;
    return accessor(f);
  }
  var pureOptions = {
    ownedWrite: true
  };
  function trySmallMove(t2, e, i, s) {
    const n = t2.ot;
    const r = t2.it - 1;
    const o = [];
    const f = [];
    const h = [];
    let c = 256;
    let a = s;
    let l = s;
    let u = false;
    while (a <= r && l <= i - 1) {
      const t3 = n[a];
      const s2 = e[l];
      if (t3 === s2) {
        if (!u) {
          h.push(a, l, 0);
          u = true;
        }
        h[h.length - 1]++;
        a++;
        l++;
        continue;
      }
      u = false;
      let d = -1;
      let w = Math.min(32 - o.length, r - a, c);
      for (let t4 = 1; t4 <= w; t4++) {
        if (n[a + t4] === s2) {
          d = t4;
          break;
        }
      }
      c -= d === -1 ? w : d;
      let p = -1;
      w = Math.min(32 - f.length, i - 1 - l, c);
      for (let i2 = 1; i2 <= w; i2++) {
        if (e[l + i2] === t3) {
          p = i2;
          break;
        }
      }
      c -= p === -1 ? w : p;
      if (d !== -1 && (p === -1 || d <= p)) {
        while (d-- > 0) o.push(a++);
        continue;
      }
      if (p !== -1) {
        while (p-- > 0) f.push(l++);
        continue;
      }
      if (c <= 0 || o.length === 32 || f.length === 32) return false;
      o.push(a++);
      f.push(l++);
    }
    for (; a <= r; a++) {
      if (o.length === 32) return false;
      o.push(a);
    }
    for (; l <= i - 1; l++) {
      if (f.length === 32) return false;
      f.push(l);
    }
    return commitSmallMove(t2, e, i, o, f, h);
  }
  function commitSmallMove(t2, e, i, s, n, r) {
    const o = t2.ot;
    let f;
    let h;
    let c;
    if (n.length !== 0) {
      c = new Array(s.length);
      for (h = 0; h < n.length; h++) {
        let t3 = -1;
        for (f = 0; f < s.length; f++) {
          if (!c[f] && o[s[f]] === e[n[h]]) {
            t3 = f;
            break;
          }
        }
        if (t3 === -1) return false;
        c[t3] = true;
        n[h] = n[h] << 6 | t3;
      }
    }
    if (s.length !== 0 || n.length !== 0) {
      const t3 = /* @__PURE__ */ new Set();
      for (f = 0; f < s.length; f++) t3.add(o[s[f]]);
      for (h = 0; h < n.length; h++) t3.add(e[n[h] >> 6]);
      for (let e2 = 0; e2 < r.length; e2 += 3) {
        const i2 = r[e2];
        for (let s2 = 0, n2 = r[e2 + 2]; s2 < n2; s2++) if (t3.has(o[i2 + s2])) return false;
      }
    }
    const a = t2.ht;
    const l = t2.tn;
    const u = a.slice(0, i);
    const d = l.slice(0, i);
    for (let t3 = 0; t3 < r.length; t3 += 3) {
      const e2 = r[t3];
      const i2 = r[t3 + 1];
      if (e2 !== i2) {
        for (let s2 = 0; s2 < r[t3 + 2]; s2++) {
          u[i2 + s2] = a[e2 + s2];
          d[i2 + s2] = l[e2 + s2];
        }
      }
    }
    for (h = 0; h < n.length; h++) {
      const t3 = n[h] >> 6;
      const e2 = s[n[h] & 63];
      u[t3] = a[e2];
      d[t3] = l[e2];
    }
    t2.ht = u;
    t2.tn = d;
    t2.it = i;
    t2.ot = e.slice(0);
    for (f = 0; f < s.length; f++) {
      if (c === void 0 || !c[f]) l[s[f]].dispose();
    }
    return true;
  }
  function stageRemoval(t2, e, i, s, n, r, o, f, h) {
    const c = t2.ze;
    const a = t2.ot;
    const l = t2.tn;
    const u = [];
    let d = i;
    for (let w = i; w <= n; d++, w++) {
      const n2 = c ? c(e[w]) : e[w];
      while (d <= s && (c ? c(a[d]) : a[d]) !== n2) u.push(l[d++]);
      if (d > s) {
        o.fill(void 0);
        return null;
      }
      r[w] = t2.ht[d];
      o[w - i] = l[d];
      if (f) f[w - i] = t2.ct[d];
      if (h) h[w - i] = t2.lt[d];
    }
    for (; d <= s; d++) u.push(l[d]);
    return u;
  }
  function commitWindow(t2, e, i, s, n, r, o, f, h) {
    const c = t2.tn;
    const a = t2.ct;
    const l = t2.lt;
    const u = t2.it;
    const d = i - u;
    let w, p;
    if (d > 0) {
      c.length = i;
      if (a) a.length = i;
      if (l) l.length = i;
      for (w = u - 1, p = i - 1; w > n; w--, p--) {
        c[p] = c[w];
        if (a) a[p] = a[w];
        if (l) l[p] = l[w];
      }
    } else if (d < 0) {
      for (w = n + 1, p = r + 1; w < u; w++, p++) {
        c[p] = c[w];
        if (a) a[p] = a[w];
        if (l) l[p] = l[w];
      }
      c.length = i;
      if (a) a.length = i;
      if (l) l.length = i;
    }
    for (p = s; p <= r; p++) {
      c[p] = o[p - s];
      if (a) setSignal(a[p] = f[p - s], e[p]);
      if (l) setSignal(l[p] = h[p - s], p);
    }
    for (p = r + 1; p < i; p++) {
      if (a) setSignal(a[p], e[p]);
      if (l && d !== 0) setSignal(l[p], p);
    }
  }
  function updateKeyedMap() {
    this.L.Ye = getOwner();
    const t2 = this.st() || [], e = t2.length;
    t2[$TRACK];
    runWithOwner(this.L, () => {
      let i, s, n, r, o, f = this.ct ? this.ut ? () => {
        r[n] = signal(t2[s], pureOptions);
        return this.ft(accessor(r[n]), s);
      } : () => {
        r[n] = signal(t2[s], pureOptions);
        o && (o[n] = signal(s, pureOptions));
        return this.ft(accessor(r[n]), o ? accessor(o[n]) : void 0);
      } : this.lt ? () => {
        const e2 = t2[s];
        o[n] = signal(s, pureOptions);
        return this.ft(e2, accessor(o[n]));
      } : () => {
        const e2 = t2[s];
        return this.ft(e2);
      };
      if (e === 0) {
        if (this.it !== 0) {
          this.L.dispose(false);
          this.tn = [];
          this.ot = [];
          this.ht = [];
          this.it = 0;
          this.ct && (this.ct = []);
          this.lt && (this.lt = []);
        }
        if (this.H && !this.ht.length) {
          this.tn[0]?.dispose();
          this.ht[0] = runWithOwner(this.tn[0] = createOwner(), this.H);
        }
      } else if (this.it === 0) {
        const h = new Array(e);
        const c = new Array(e);
        r = this.ct && new Array(e);
        o = this.lt && new Array(e);
        try {
          for (s = 0; s < e; s++) {
            n = s;
            h[s] = runWithOwner(c[s] = createOwner(), f);
          }
        } catch (t3) {
          for (i = 0; i <= s; i++) c[i]?.dispose();
          throw t3;
        }
        if (this.tn[0]) this.tn[0].dispose();
        this.ht = h;
        this.tn = c;
        r && (this.ct = r);
        o && (this.lt = o);
        this.ot = t2.slice(0);
        this.it = e;
      } else {
        let h, c, a, l, u, d, w, p, m, O, _;
        const y = setSignal;
        for (h = 0, c = Math.min(this.it, e); h < c && (this.ot[h] === t2[h] || this.ct && compare(this.ze, this.ot[h], t2[h])); h++) {
          if (this.ct) y(this.ct[h], t2[h]);
        }
        for (c = this.it - 1, a = e - 1; c >= h && a >= h && (this.ot[c] === t2[a] || this.ct && compare(this.ze, this.ot[c], t2[a])); c--, a--) ;
        if (h === e && this.it === e) {
          this.ot = t2.slice(0);
          return;
        }
        if (e <= this.it && c - h > 64 && this.ct === void 0 && this.lt === void 0) {
          const i2 = h + (a - h >> 1);
          const s2 = t2[i2];
          const n2 = Math.min(c, i2 + 32);
          let r2 = Math.max(h, i2 - 32);
          while (r2 <= n2 && this.ot[r2] !== s2) r2++;
          if (r2 <= n2 && trySmallMove(this, t2, e, h)) return;
        }
        const g = e - this.it;
        const A = a - h + 1;
        const S = new Array(e);
        const M = new Array(A);
        r = this.ct ? new Array(A) : void 0;
        o = this.lt ? new Array(A) : void 0;
        const k = g < 0 ? stageRemoval(this, t2, h, c, a, S, M, r, o) : null;
        if (k !== null) {
          if (k.length !== 0) p = k;
        } else {
          d = /* @__PURE__ */ new Map();
          w = new Array(A);
          for (s = a; s >= h; s--) {
            l = t2[s];
            u = this.ze ? this.ze(l) : l;
            i = d.get(u);
            w[s - h] = i === void 0 ? -1 : i;
            d.set(u, s);
          }
          for (i = h; i <= c; i++) {
            l = this.ot[i];
            u = this.ze ? this.ze(l) : l;
            s = d.get(u);
            if (s !== void 0 && s !== -1) {
              S[s] = this.ht[i];
              M[s - h] = this.tn[i];
              r && (r[s - h] = this.ct[i]);
              o && (o[s - h] = this.lt[i]);
              s = w[s - h];
              d.set(u, s);
            } else {
              (p ??= []).push(this.tn[i]);
              if (false) ;
            }
          }
          try {
            for (s = h; s <= a; s++) {
              if (M[s - h] !== void 0) continue;
              (m ??= []).push(M[s - h] = createOwner());
              if (false) ;
              n = s - h;
              S[s] = runWithOwner(M[s - h], f);
            }
          } catch (t3) {
            if (m) for (i = 0; i < m.length; i++) m[i].dispose();
            throw t3;
          }
        }
        for (i = 0; i < h; i++) S[i] = this.ht[i];
        for (s = a + 1; s < e; s++) S[s] = this.ht[s - g];
        commitWindow(this, t2, e, h, c, a, M, r, o);
        this.ht = S;
        this.it = e;
        this.ot = t2.slice(0);
        if (p) for (i = 0; i < p.length; i++) p[i].dispose();
        if (false) ;
      }
    });
    return this.ht;
  }
  function compare(t2, e, i) {
    return t2 ? t2(e) === t2(i) : true;
  }

  // node_modules/.pnpm/@solidjs+signals@2.0.0-rc.17/node_modules/@solidjs/signals/dist/prod/flatten.js
  function flatten(r, t2) {
    if (typeof r === "function" && !r.length) {
      if (t2?.doNotUnwrap) return r;
      do {
        r = r();
      } while (typeof r === "function" && !r.length);
    }
    if (t2?.skipNonRendered && (r == null || r === true || r === false || r === "")) return;
    if (Array.isArray(r)) {
      let e = [];
      if (flattenArray(r, e, t2)) {
        return () => {
          let r2 = [];
          flattenArray(e, r2, {
            ...t2,
            doNotUnwrap: false
          });
          return r2;
        };
      }
      return e;
    }
    return r;
  }
  function flattenArray(r, t2 = [], e) {
    let o = null;
    let f = false;
    for (let n = 0; n < r.length; n++) {
      try {
        let o2 = r[n];
        if (typeof o2 === "function" && !o2.length) {
          if (e?.doNotUnwrap) {
            t2.push(o2);
            f = true;
            continue;
          }
          do {
            o2 = o2();
          } while (typeof o2 === "function" && !o2.length);
        }
        if (Array.isArray(o2)) {
          f = flattenArray(o2, t2, e) || f;
        } else if (e?.skipNonRendered && (o2 == null || o2 === true || o2 === false || o2 === "")) {
        } else t2.push(o2);
      } catch (r2) {
        if (!(r2 instanceof NotReadyError)) throw r2;
        o = r2;
      }
    }
    if (o) throw o;
    return f;
  }

  // node_modules/.pnpm/@solidjs+signals@2.0.0-rc.17/node_modules/@solidjs/signals/dist/prod/boundaries.js
  var BOUNDARY = Symbol("");
  var REVEAL = Symbol("");
  var revealHooks = null;
  var collecting = /* @__PURE__ */ new Set();
  var boundaryOf = (e) => e.A[BOUNDARY];
  var isCollecting = (e) => e.i === STATUS_ERROR || !e.D || e.h;
  function record(e, n) {
    e.U.add(n);
    collecting.add(e);
  }
  function caught(e, n, r) {
    if (e.i === STATUS_ERROR) {
      r ??= n.T?.F;
      n = r?.source ?? n;
      const t2 = unwrapStatusError(r);
      setSignal(e.F, t2);
      reportClientError(t2, e.L, n);
    }
    record(e, n);
    if (e.h) flip(e);
    if (!e.H) redraw(e);
    if (e.V !== null) revealHooks.P(e.V);
  }
  function flip(e) {
    e.D = false;
    e.v = e.B;
    const n = e.j;
    if (!e.v && n.l & CONFIG_HELD && txOf(n) !== flushTransaction) unhold(n);
    if (e.v !== null && e.j !== null) laneStageHook(e.j, e.v, true, false);
  }
  function redraw(e) {
    if (e.j !== null && !(e.j.R & (REACTIVE_DISPOSED | REACTIVE_ZOMBIE))) {
      enqueueSub(e.j);
      schedule();
    }
  }
  function catchStatus(e, n, r) {
    if (n === 0) {
      if (collecting.size !== 0) {
        for (let n2 = boundaryOf(e); n2; n2 = n2._parent) if (!(e.m & unsettled(n2)) && n2.U.delete(e) && n2.U.size === 0) {
          if (n2.H) redraw(n2);
          if (n2.V !== null) revealHooks.P(n2.V);
        }
      }
      return false;
    }
    for (let t2 = boundaryOf(e); t2; t2 = t2._parent) {
      if (!(t2.i & n)) continue;
      if (isCollecting(t2)) {
        caught(t2, e, r);
        return t2.h ? CAUGHT_REARMED : true;
      }
      record(t2, e);
    }
    return false;
  }
  function armedOver(e) {
    for (let n = boundaryOf(e); n; n = n._parent) if (n.i & STATUS_PENDING && isCollecting(n)) return n.h;
  }
  function hiddenBy(e) {
    for (let n = boundaryOf(e); n; n = n._parent) if (n.H) return n;
    return void 0;
  }
  function heldRun(e) {
    const n = hiddenBy(e);
    if (n === void 0) return false;
    (n.Z ??= /* @__PURE__ */ new Set()).add(e);
    return true;
  }
  function release(e) {
    if (e.Z === null) return;
    for (const n of e.Z) {
      const e2 = n.i;
      globalQueue.enqueue(e2 === EFFECT_TRACKED ? EFFECT_USER : e2, e2 === EFFECT_TRACKED ? n.M : n.W ??= GlobalQueue.K.bind(null, n));
    }
    e.Z = null;
  }
  var unsettled = (e) => e.i === STATUS_ERROR ? STATUS_ERROR | STATUS_PENDING : STATUS_PENDING;
  function prune(e, n) {
    const r = unsettled(e);
    for (const t2 of e.U) {
      if (t2.R & REACTIVE_DISPOSED) e.U.delete(t2);
      else if (!(t2.m & r)) {
        if ((t2 !== e.Y || e.h) && (t2.l & (CONFIG_HELD | CONFIG_OVERRIDE)) === CONFIG_HELD) {
          if (!n || !e.D) continue;
          joinFuture(txOf(t2));
        }
        e.U.delete(t2);
      }
    }
    return e.U.size;
  }
  function boundarySeam() {
    for (const e of collecting) {
      const n = e.j.R;
      if (n & REACTIVE_DISPOSED) {
        collecting.delete(e);
        continue;
      }
      if (n & REACTIVE_ZOMBIE) continue;
      prune(e, false);
      const r = e.j;
      if (e.h && !e.D && r.l & CONFIG_HELD && [...e.U].every((e2) => !(e2.m & STATUS_PENDING) && e2.l & CONFIG_HELD && txOf(e2) === txOf(r))) {
        e.U.clear();
        e.D = true;
      }
      e.h = false;
      if (e.U.size === 0) {
        collecting.delete(e);
        if (e.H) redraw(e);
        if (e.V !== null) revealHooks.P(e.V);
      }
    }
  }
  function arm(e) {
    if (!e.D) {
      if (e.H && e.v === null && e.B === null && passLane !== null) {
        e.B = passLane;
        flip(e);
        redraw(e);
      }
      return;
    }
    e.h = true;
    let n = passLane;
    if (n === null && globalQueue.q && getOwner().l & CONFIG_VERDICT) {
      joinFuture(null);
      n = GlobalQueue.J(flushTransaction);
    }
    e.B = n;
    collecting.add(e);
    const r = e.U.size;
    heldUnder(e.L, flushTransaction, (n2) => record(e, n2));
    for (const n2 of e.U) {
      if (e.U.size !== r || n2.m & STATUS_PENDING && !(n2.R & REACTIVE_DISPOSED)) {
        for (let n3 = e._parent; n3 !== null; n3 = n3._parent) for (const r2 of e.U) n3.U.delete(r2);
        flip(e);
        redraw(e);
        return;
      }
    }
  }
  function reset(e) {
    for (const n of [...e.U]) if (n.X !== void 0 && !(n.R & REACTIVE_DISPOSED)) recompute(n);
    schedule();
  }
  function createBoundary(e, n, r, t2) {
    const o = createOwner();
    const l = {
      i: e,
      _parent: boundaryOf(o) ?? null,
      L: o,
      Y: null,
      j: null,
      U: /* @__PURE__ */ new Set(),
      D: false,
      h: false,
      B: null,
      v: null,
      H: false,
      Z: null,
      V: null,
      $: false,
      ee: false,
      F: e === STATUS_ERROR ? signal(void 0, ERROR_SIGNAL) : null,
      ne: r,
      re: null
    };
    const u = e === STATUS_PENDING ? o.A[REVEAL] : null;
    if (getOwner()) cleanup(() => {
      collecting.delete(l);
      if (u) revealHooks.te(u, l);
    });
    if (t2) {
      let e2 = false;
      runWithOwner(o, () => computed(() => {
        try {
          t2();
        } catch (e3) {
          if (!(e3 instanceof NotReadyError)) {
            if (!globalQueue.notify(getOwner(), STATUS_ERROR, STATUS_ERROR, e3)) {
              if (false) ;
              else haltReactivity(e3);
              throw e3;
            }
            return;
          }
        }
        if (e2) arm(l);
        else e2 = true;
      })).l |= CONFIG_REDERIVE;
    }
    const a = {
      ...o.A,
      [BOUNDARY]: l
    };
    if (revealHooks !== null) a[REVEAL] = null;
    o.A = a;
    const i = runWithOwner(o, () => {
      const e2 = false ? computed(n, {
        name: "children"
      }) : computed(n);
      return false ? computed(() => flatten(read(e2)), {
        name: "boundary"
      }) : computed(() => flatten(read(e2)));
    });
    l.Y = i;
    if (u) revealHooks.oe(u, l);
    const f = computed(
      () => {
        if (l.$) {
          link(i, getOwner());
          if (l.V !== null) revealHooks.P(l.V);
          if (l.$) {
            if (l.ee) {
              l.H = true;
              return void 0;
            }
            return fallback(l);
          }
        }
        const n2 = i.m & (STATUS_PENDING | STATUS_ERROR);
        if (n2 !== 0) {
          link(i, getOwner());
          if (n2 & e) {
            if (isCollecting(l)) {
              caught(l, i, void 0);
              return fallback(l);
            }
            record(l, i);
          }
          throw i.T.F;
        }
        if (isCollecting(l)) {
          const e2 = getOwner();
          if (l.U.size !== 0 && e2.m & STATUS_UNINITIALIZED && !(e2.l & CONFIG_HELD)) return fallback(l);
          if (prune(l, true) !== 0) return fallback(l);
        }
        let r2;
        try {
          r2 = read(i);
        } catch (n3) {
          if (n3 instanceof NotReadyError ? e === STATUS_PENDING : e === STATUS_ERROR) {
            if (isCollecting(l)) {
              caught(l, i, n3);
              return fallback(l);
            }
            record(l, i);
          }
          throw n3;
        }
        if (isCollecting(l) && l.U.size !== 0) return fallback(l);
        l.D = true;
        l.H = false;
        const t3 = l.re;
        if (t3 !== null) globalQueue.enqueue(EFFECT_RENDER, () => {
          if (!l.H && l.re === t3) {
            dispose(t3);
            l.re = null;
          }
        });
        release(l);
        if (l.V !== null) {
          const e2 = l.V;
          l.V = null;
          revealHooks.P(e2);
        }
        return r2;
      },
      // Boundary structure, not a user source: its value is fallback-or-content
      // and legitimately swaps mid-hydration (resume), so it must never be
      // frozen by snapshot capture.
      {
        le: true
      }
    );
    f.l |= CONFIG_REDERIVE;
    l.j = f;
    return accessor(f);
  }
  function fallback(e) {
    if (e.v !== null) {
      setPassLane(e.v);
      e.j.R |= REACTIVE_LANE_READ;
      e.v = null;
    }
    e.H = true;
    if (e.re === null) {
      const n = getOwner();
      e.re = runWithOwner(n._parent, () => computed(() => e.ne(e), {
        id: n.id
      }));
    }
    return read(e.re);
  }
  var ERROR_SIGNAL = {
    ownedWrite: true,
    le: true
  };
  function createErrorBoundary(e, n) {
    return createBoundary(STATUS_ERROR, e, (e2) => n(accessor(e2.F), () => reset(e2)));
  }
  installBoundaryHooks(catchStatus, (e, n) => !n && (e.m & STATUS_UNINITIALIZED ? catchStatus(e, STATUS_PENDING) : armedOver(e)), hiddenBy, boundarySeam, heldRun);

  // node_modules/.pnpm/@solidjs+signals@2.0.0-rc.17/node_modules/@solidjs/signals/dist/prod/store/utils.js
  function trueFn() {
    return true;
  }
  var SOURCE_PLAIN = 0;
  var SOURCE_OMIT = 1;
  var SOURCE_PROXY = 2;
  var SOURCE_MEMO = 3;
  var SOURCE_MERGE = 4;
  var EMPTY = Object.freeze({});
  function leafOf(e, n) {
    return n === SOURCE_MEMO ? (e = e()) == null ? EMPTY : e : e;
  }
  var OmitView = class {
    source;
    kind;
    hidden;
    /** see `MergeView.table` */
    table = 0;
    /** see `tableOwnKeys` / `tableDescriptor` */
    keys = void 0;
    descs = void 0;
    constructor(e, n, t2) {
      this.source = e;
      this.kind = n;
      this.hidden = t2;
    }
  };
  function isHidden(e, n) {
    const t2 = e.hidden;
    return typeof t2 === "function" ? t2(n) : t2.includes(n);
  }
  function viewSource(e) {
    return leafOf(e.source, e.kind);
  }
  function recordOf(e) {
    return e[$RECORD];
  }
  function leafKeys(e, n) {
    if (n === SOURCE_PLAIN) return Object.keys(e);
    if (n === SOURCE_PROXY || e[$PROXY] === e) return Reflect.ownKeys(e);
    return Object.keys(e);
  }
  function sourceKeys(e, n) {
    if (n === SOURCE_OMIT) {
      if (e.kind === SOURCE_MERGE) return mergeKeysOf(e.source, false, e);
      const n2 = leafKeys(viewSource(e), e.kind);
      const t2 = [];
      for (let r = 0; r < n2.length; r++) if (!isHidden(e, n2[r])) t2.push(n2[r]);
      return t2;
    }
    return leafKeys(leafOf(e, n), n);
  }
  function sourceHas(e, n, t2) {
    if (n === SOURCE_OMIT) {
      if (isHidden(e, t2)) return false;
      return e.kind === SOURCE_MERGE ? mergeHas(e.source, t2) : t2 in viewSource(e);
    }
    return t2 in leafOf(e, n);
  }
  function sourceGet(e, n, t2) {
    if (n === SOURCE_OMIT) {
      if (isHidden(e, t2)) return void 0;
      return e.kind === SOURCE_MERGE ? mergeGet(e.source, t2) : viewSource(e)[t2];
    }
    return leafOf(e, n)[t2];
  }
  function entryHasStaticKeys(e, n) {
    if (n === SOURCE_PLAIN) return true;
    if (n !== SOURCE_OMIT) return false;
    if (e.kind === SOURCE_PLAIN) return true;
    return e.kind === SOURCE_MERGE && mergeHasStaticKeys(e.source);
  }
  function mergeHasStaticKeys(e) {
    const n = e.sources, t2 = e.kinds;
    for (let e2 = 0; e2 < n.length; e2++) if (!entryHasStaticKeys(n[e2], t2[e2])) return false;
    return true;
  }
  function hasStaticKeys(e) {
    if (!($PROXY in e)) return true;
    const n = recordOf(e);
    if (n instanceof MergeView) return mergeHasStaticKeys(n);
    return n !== void 0 && entryHasStaticKeys(n, SOURCE_OMIT);
  }
  function accessorDescriptor(e, n = true) {
    return {
      configurable: true,
      enumerable: n,
      get: e,
      set: trueFn
    };
  }
  function sourceDescriptor(e, n, t2, r = false) {
    if (n === SOURCE_OMIT) {
      if (isHidden(e, t2)) return void 0;
      return sourceDescriptor(e.source, e.kind, t2, r);
    }
    if (n === SOURCE_MERGE) return mergeDescriptor(e, t2);
    if (n === SOURCE_MEMO) {
      return r || t2 in leafOf(e, n) ? accessorDescriptor(() => leafOf(e, n)[t2]) : void 0;
    }
    if (n === SOURCE_PROXY) {
      if (recordOf(e) !== void 0) return Reflect.getOwnPropertyDescriptor(e, t2);
      return r || t2 in e ? accessorDescriptor(() => e[t2]) : void 0;
    }
    const i = Reflect.getOwnPropertyDescriptor(e, t2);
    if (i === void 0) return void 0;
    if (i.get !== void 0 || i.set !== void 0) return accessorDescriptor(() => e[t2], i.enumerable);
    if (i.configurable) return i;
    return {
      configurable: true,
      enumerable: i.enumerable,
      writable: true,
      value: i.value
    };
  }
  var MergeView = class {
    sources;
    kinds;
    /** key → the plain leaf that owns it (later sources win), built by an
     * enumeration or once the reads have paid for it (see `resolvedTable`)
     * when every leaf has static keys; `null` when one doesn't. Until then
     * the slot counts the per-key trap reads so far. One slot rather than a
     * counter field of its own: a view is built per source per component
     * layer, and each field initializer is a measurable share of a
     * constructor that small in the lower JIT tiers. */
    table = 0;
    /** see `tableOwnKeys` / `tableDescriptor` */
    keys = void 0;
    descs = void 0;
    constructor(e, n) {
      this.sources = e;
      this.kinds = n;
    }
  };
  function viewOf(e) {
    return e != null && $PROXY in e ? e[$RECORD] : void 0;
  }
  function resolvedTable(e) {
    if (e == null || !($PROXY in e)) return void 0;
    const n = recordOf(e);
    if (n instanceof OmitView) return omitTable(n);
    return n === void 0 ? void 0 : mergeTable(n);
  }
  function mergeTable(e) {
    let n = e.table;
    if (typeof n !== "object") {
      const t2 = e.sources, r = e.kinds;
      for (let n2 = 0; n2 < t2.length; n2++) {
        if (!entryHasStaticKeys(t2[n2], r[n2])) {
          e.table = null;
          return void 0;
        }
      }
      n = /* @__PURE__ */ new Map();
      collectTable(n, e, void 0);
      e.table = n;
    }
    return n === null ? void 0 : n;
  }
  function collectTable(e, n, t2) {
    const r = n.sources, i = n.kinds;
    for (let n2 = 0; n2 < r.length; n2++) {
      const f = r[n2];
      if (i[n2] === SOURCE_OMIT) {
        if (f.kind === SOURCE_MERGE) {
          if (t2 === void 0) t2 = [f];
          else t2.push(f);
          collectTable(e, f.source, t2);
          t2.pop();
          continue;
        }
        const n3 = f.source;
        const r2 = Reflect.ownKeys(n3);
        for (let i2 = 0; i2 < r2.length; i2++) {
          const u = r2[i2];
          if (!isHidden(f, u) && !hiddenByAny(t2, u)) tableSet(e, u, n3);
        }
      } else {
        const n3 = Reflect.ownKeys(f);
        for (let r2 = 0; r2 < n3.length; r2++) {
          const i2 = n3[r2];
          if (!hiddenByAny(t2, i2)) tableSet(e, i2, f);
        }
      }
    }
  }
  function hiddenByAny(e, n) {
    if (e !== void 0) {
      for (let t2 = e.length - 1; t2 >= 0; t2--) if (isHidden(e[t2], n)) return true;
    }
    return false;
  }
  function tableSet(e, n, t2) {
    if (e.has(n)) e.delete(n);
    e.set(n, t2);
  }
  function omitTable(e) {
    let n = e.table;
    if (typeof n !== "object") {
      const t2 = e.source;
      if (e.kind === SOURCE_MERGE) {
        if (!mergeHasStaticKeys(t2)) {
          e.table = null;
          return void 0;
        }
        n = /* @__PURE__ */ new Map();
        collectTable(n, t2, [e]);
      } else if (e.kind === SOURCE_PLAIN) {
        n = /* @__PURE__ */ new Map();
        const r = Reflect.ownKeys(t2);
        for (let i = 0; i < r.length; i++) {
          const f = r[i];
          if (!isHidden(e, f)) n.set(f, t2);
        }
      } else {
        e.table = null;
        return void 0;
      }
      e.table = n;
    }
    return n === null ? void 0 : n;
  }
  var propertyIsEnumerable = Object.prototype.propertyIsEnumerable;
  function tableOwnKeys(e, n) {
    let t2 = e.keys;
    if (t2 === void 0) {
      t2 = e.keys = [];
      for (const [e2, r] of n) if (propertyIsEnumerable.call(r, e2)) t2.push(e2);
    }
    return t2;
  }
  function tableDescriptor(e, n, t2) {
    const r = n.get(t2);
    if (r === void 0) return void 0;
    let i = e.descs;
    if (i === void 0) i = e.descs = /* @__PURE__ */ new Map();
    let f = i.get(t2);
    if (f === void 0) {
      f = sourceDescriptor(r, SOURCE_PLAIN, t2);
      if (f === void 0) return void 0;
      i.set(t2, f);
      return f;
    }
    if (f.get !== void 0) return f;
    return {
      configurable: true,
      enumerable: f.enumerable,
      writable: f.writable,
      value: r[t2]
    };
  }
  var READS_FOR_TABLE = 16;
  function mergeReadTable(e) {
    const n = e.table;
    if (typeof n === "object") return n === null ? void 0 : n;
    if (n + 1 < READS_FOR_TABLE) {
      e.table = n + 1;
      return void 0;
    }
    return mergeTable(e);
  }
  function tableOf(e) {
    const n = e.table;
    return typeof n === "object" && n !== null ? n : void 0;
  }
  var MISSING = Symbol();
  function mergeLookup(e, n) {
    const t2 = tableOf(e);
    if (t2 !== void 0) {
      const e2 = t2.get(n);
      return e2 === void 0 ? MISSING : e2[n];
    }
    const r = e.sources, i = e.kinds;
    for (let e2 = r.length - 1; e2 >= 0; e2--) {
      const t3 = i[e2];
      if (t3 === SOURCE_PLAIN) {
        const t4 = r[e2];
        if (n in t4) return t4[n];
        continue;
      }
      if (t3 === SOURCE_OMIT) {
        const t4 = r[e2];
        if (isHidden(t4, n)) continue;
        if (t4.kind === SOURCE_MERGE) {
          const e3 = mergeLookup(t4.source, n);
          if (e3 !== MISSING) return e3;
          continue;
        }
        const i2 = viewSource(t4);
        if (n in i2) return i2[n];
      } else {
        const i2 = leafOf(r[e2], t3);
        if (n in i2) return i2[n];
      }
    }
    return MISSING;
  }
  function mergeGet(e, n) {
    const t2 = mergeLookup(e, n);
    return t2 === MISSING ? void 0 : t2;
  }
  function mergeHas(e, n) {
    const t2 = tableOf(e);
    if (t2 !== void 0) return t2.has(n);
    const r = e.sources, i = e.kinds;
    for (let e2 = r.length - 1; e2 >= 0; e2--) if (sourceHas(r[e2], i[e2], n)) return true;
    return false;
  }
  function mergeDescriptor(e, n) {
    const t2 = tableOf(e);
    if (t2 !== void 0) return tableDescriptor(e, t2, n);
    const r = e.sources, i = e.kinds;
    for (let t3 = r.length - 1; t3 >= 0; t3--) {
      if (!sourceHas(r[t3], i[t3], n)) continue;
      return sourceDescriptor(r[t3], i[t3], n, true) ?? accessorDescriptor(() => mergeGet(e, n));
    }
    return void 0;
  }
  function mergeKeysOf(e, n, t2) {
    const r = [];
    collectKeys(e, t2 === void 0 ? void 0 : [t2], n, r, null);
    return r;
  }
  function collectKeys(e, n, t2, r, i) {
    const f = e.sources, u = e.kinds;
    for (let e2 = 0; e2 < f.length; e2++) {
      let o = f[e2], c = u[e2];
      let s;
      if (c === SOURCE_OMIT) {
        if (o.kind === SOURCE_MERGE) {
          if (n === void 0) n = [o];
          else n.push(o);
          collectKeys(o.source, n, t2, r, i);
          n.pop();
          continue;
        }
        s = o;
        c = o.kind;
        o = o.source;
      }
      o = leafOf(o, c);
      const d = t2 ? ownEnumerableKeys(o) : leafKeys(o, c);
      for (let e3 = 0; e3 < d.length; e3++) {
        const t3 = d[e3];
        if (s !== void 0 && isHidden(s, t3)) continue;
        if (hiddenByAny(n, t3)) continue;
        addKey(r, i, t3, o);
      }
    }
  }
  function addKey(e, n, t2, r) {
    const i = e.indexOf(t2);
    if (i !== -1) {
      e.splice(i, 1);
      if (n !== null) n.splice(i, 1);
    }
    e.push(t2);
    if (n !== null) n.push(r);
  }
  function mergeEnumerableKeys(e, n) {
    const t2 = mergeTable(e);
    if (t2 === void 0) return mergeKeysOf(e, true, n);
    const r = tableOwnKeys(e, t2);
    if (n === void 0) return r;
    const i = [];
    for (let e2 = 0; e2 < r.length; e2++) if (!isHidden(n, r[e2])) i.push(r[e2]);
    return i;
  }
  var mergeTraps = {
    get(e, n, t2) {
      if (typeof n === "symbol") {
        if (n === $PROXY) return t2;
        if (n === $RECORD) return e;
        if (n === $TARGET) return void 0;
      }
      const r = e.table;
      let i;
      if (typeof r !== "object") {
        if (r + 1 < READS_FOR_TABLE) {
          e.table = r + 1;
          const t3 = e.sources, i2 = e.kinds;
          for (let r2 = t3.length - 1; r2 >= 0; r2--) {
            if (i2[r2] !== SOURCE_PLAIN) return mergeGet(e, n);
            const f2 = t3[r2][n];
            if (f2 !== void 0 || n in t3[r2]) return f2;
          }
          return void 0;
        }
        i = mergeTable(e);
        if (i === void 0) return mergeGet(e, n);
      } else if (r === null) return mergeGet(e, n);
      else i = r;
      const f = i.get(n);
      return f === void 0 ? void 0 : f[n];
    },
    has(e, n) {
      if (n === $PROXY) return true;
      if (n === $TARGET || n === $RECORD) return false;
      const t2 = mergeReadTable(e);
      if (t2 !== void 0) return t2.has(n);
      return mergeHas(e, n);
    },
    set: trueFn,
    deleteProperty: trueFn,
    getOwnPropertyDescriptor(e, n) {
      if (n === $PROXY || n === $TARGET || n === $RECORD) return void 0;
      const t2 = mergeReadTable(e);
      if (t2 !== void 0) return tableDescriptor(e, t2, n);
      return mergeDescriptor(e, n);
    },
    ownKeys(e) {
      return mergeEnumerableKeys(e);
    }
  };
  function merge2(...e) {
    if (e.length === 1 && typeof e[0] !== "function") return e[0];
    const n = new Array(e.length);
    const t2 = new Array(e.length);
    let r = 0;
    let i = void 0;
    let f = 0;
    for (let u2 = 0; u2 < e.length; u2++) {
      const o2 = e[u2];
      if (!o2) continue;
      f++;
      i = o2;
      if (typeof o2 === "function") {
        n[r] = createMemo(o2);
        t2[r++] = SOURCE_MEMO;
        continue;
      }
      if ($PROXY in o2) {
        const e2 = recordOf(o2);
        if (e2 instanceof MergeView) {
          for (let i2 = 0; i2 < e2.sources.length; i2++) {
            n[r] = e2.sources[i2];
            t2[r++] = e2.kinds[i2];
          }
        } else if (e2 !== void 0) {
          n[r] = e2;
          t2[r++] = SOURCE_OMIT;
        } else {
          n[r] = o2;
          t2[r++] = SOURCE_PROXY;
        }
        continue;
      }
      n[r] = o2;
      t2[r++] = SOURCE_PLAIN;
    }
    if (r !== n.length) {
      n.length = r;
      t2.length = r;
    }
    if (SUPPORTS_PROXY) {
      if (f === 1 && typeof i !== "function") return i;
      return new Proxy(new MergeView(n, t2), mergeTraps);
    }
    const u = /* @__PURE__ */ Object.create(null);
    let o = false;
    let c = n.length - 1;
    for (let e2 = c; e2 >= 0; e2--) {
      const t3 = n[e2];
      if (!t3) {
        e2 === c && c--;
        continue;
      }
      const r2 = Object.getOwnPropertyNames(t3);
      for (let n2 = r2.length - 1; n2 >= 0; n2--) {
        const i2 = r2[n2];
        if (i2 === "__proto__" || i2 === "constructor") continue;
        if (!u[i2]) {
          o = o || e2 !== c;
          const n3 = Object.getOwnPropertyDescriptor(t3, i2);
          u[i2] = n3.get ? {
            enumerable: true,
            configurable: true,
            get: n3.get.bind(t3)
          } : n3;
        }
      }
    }
    if (!o) return n[c];
    const s = {};
    const d = Object.keys(u);
    for (let e2 = d.length - 1; e2 >= 0; e2--) {
      const n2 = d[e2], t3 = u[n2];
      if (t3.get) Object.defineProperty(s, n2, t3);
      else s[n2] = t3.value;
    }
    return s;
  }

  // node_modules/.pnpm/solid-js@2.0.0-rc.17/node_modules/solid-js/dist/solid.js
  var IS_DEV = false;
  var $DEVCOMP = Symbol(0);
  function createContext2(defaultValue, options3) {
    const id = Symbol(options3 && options3.name || "");
    function provider(props) {
      return createRoot(() => {
        setContext(provider, props.value);
        return children(() => props.children);
      });
    }
    provider.id = id;
    provider.defaultValue = defaultValue;
    return provider;
  }
  function children(fn2) {
    const c = createMemo(fn2, {
      lazy: true
    });
    const memo3 = createMemo(() => flatten(c()), {
      lazy: true,
      sync: true
    });
    memo3.toArray = () => {
      const v = memo3();
      return Array.isArray(v) ? v : v != null ? [v] : [];
    };
    return memo3;
  }
  var NoHydrateContext = {
    id: Symbol("NoHydrateContext"),
    defaultValue: false
  };
  var sharedConfig = {
    hydrating: false,
    registry: void 0,
    done: false
  };
  var _createRoot;
  var _createMemo;
  var _createSignal;
  var _createErrorBoundary;
  var _createRenderEffect;
  var LIVE_SOURCE = Symbol.for("solid.LiveSource");
  var LIVE_RESUME_FROM = Symbol.for("solid.LiveResumeFrom");
  var LIVE_LOCAL = Symbol.for("solid.LiveLocal");
  var createMemo2 = (...args) => {
    return (_createMemo || createMemo)(...args);
  };
  var createSignal2 = (...args) => {
    return (_createSignal || createSignal)(...args);
  };
  var createErrorBoundary2 = (...args) => (_createErrorBoundary || createErrorBoundary)(...args);
  var createRoot2 = (...args) => (_createRoot || createRoot)(...args);
  var createRenderEffect2 = (...args) => (_createRenderEffect || createRenderEffect)(...args);
  function createComponent(Comp, props, name, loc) {
    return untrack(() => Comp(props || {}));
  }
  var narrowedError = (name) => `Stale read from <${name}>.`;
  function For(props) {
    const options3 = "fallback" in props ? {
      keyed: props.keyed,
      fallback: () => props.fallback
    } : {
      keyed: props.keyed
    };
    const owner = getOwner();
    let mapped;
    const create = () => runWithOwner(owner, () => mapArray(() => props.each, props.children, options3));
    if (sharedConfig.hydrating) mapped = create();
    const list2 = () => (mapped ?? (mapped = create()))();
    return list2;
  }
  function Show(props) {
    const keyed = props.keyed;
    const conditionValue = createMemo(() => props.when, void 0);
    const condition = keyed ? conditionValue : createMemo(conditionValue, {
      equals: (a, b) => !a === !b,
      sync: true
    });
    let shown = false;
    return createMemo(() => {
      const c = condition();
      shown = c;
      if (c) {
        const child = props.children;
        const fn2 = typeof child === "function" && child.length > 0;
        return fn2 ? keyed ? untrack(() => child(c), IS_DEV) : untrack(() => child(() => {
          if (!shown) throw narrowedError("Show");
          return conditionValue();
        }), IS_DEV) : child;
      }
      return props.fallback;
    }, {
      sync: true
    });
  }
  function Errored(props) {
    return createErrorBoundary2(() => props.children, (err, reset2) => {
      const f = props.fallback;
      return typeof f === "function" ? f(err, reset2) : f;
    });
  }

  // node_modules/.pnpm/@solidjs+universal@2.0.0-rc.17_solid-js@2.0.0-rc.17/node_modules/@solidjs/universal/dist/universal.js
  var transparentOptions = {
    transparent: true,
    sync: true
  };
  var syncOptions = {
    sync: true
  };
  var effect2 = (fn2, effectFn, options3) => createRenderEffect2(fn2, effectFn, options3 ? {
    sync: true,
    ...options3,
    transparent: !options3.scope
  } : transparentOptions);
  var memo = (fn2) => createMemo2(() => fn2(), syncOptions);
  var named = (options3, fallback2) => options3;
  var INNER_OWNED = {};
  function createRenderer({
    createElement: createElement2,
    createTextNode: createTextNode2,
    createSentinel = () => createTextNode2(""),
    isTextNode,
    replaceText,
    insertNode: insertNode2,
    removeNode: removeNode2,
    cleanupNodes,
    setProperty,
    getParentNode,
    getFirstChild,
    getNextSibling
  }) {
    function insert2(parent, accessor2, marker, initial, options3) {
      const onUpdate = options3 && options3.onUpdate;
      let effectOptions = options3;
      if (onUpdate) {
        const {
          onUpdate: onUpdate2,
          ...rest
        } = options3;
        effectOptions = rest;
      }
      effectOptions = named(effectOptions);
      const multi = marker !== void 0;
      if (multi && !initial) initial = [];
      if (typeof accessor2 !== "function") {
        accessor2 = normalize(accessor2, multi, true);
        if (typeof accessor2 !== "function") {
          insertExpression(parent, accessor2, initial, marker);
          onUpdate && onUpdate(accessor2);
          return;
        }
      }
      if (multi && initial.length === 0) {
        const sentinel = createSentinel();
        insertNode2(parent, sentinel, marker);
        initial = [sentinel];
      }
      let current = initial;
      effect2((prev) => {
        const value = normalize(accessor2(), multi, true);
        if (typeof value !== "function") return value;
        effect2(() => normalize(value, multi), (inner) => {
          insertExpression(parent, inner, current, marker);
          current = inner;
          onUpdate && onUpdate(current);
        }, prev !== void 0 && !(options3 && options3.schedule) ? {
          ...effectOptions,
          schedule: true
        } : effectOptions);
        return INNER_OWNED;
      }, (value) => {
        if (value === INNER_OWNED) return;
        insertExpression(parent, value, current, marker);
        current = value;
        onUpdate && onUpdate(current);
      }, effectOptions);
    }
    function insertExpression(parent, value, current, marker) {
      if (value === current) return;
      const t2 = typeof value, multi = marker !== void 0;
      if (t2 === "string" || t2 === "number") {
        const tc = typeof current;
        if (tc === "string" || tc === "number") {
          replaceText(getFirstChild(parent), value);
        } else {
          cleanChildren(parent, current, marker, createTextNode2(value));
        }
      } else if (value == null) {
        cleanChildren(parent, current, marker);
      } else if (Array.isArray(value)) {
        if (value.length === 0) {
          cleanChildren(parent, current, marker);
        } else {
          if (Array.isArray(current)) {
            if (current.length === 0) {
              appendNodes(parent, value, marker);
            } else reconcileArrays(parent, current, value);
          } else if (current == null) {
            appendNodes(parent, value);
          } else {
            reconcileArrays(parent, multi && current || [getFirstChild(parent)], value);
          }
        }
      } else {
        if (Array.isArray(current)) {
          cleanChildren(parent, current, multi ? marker : null, value);
        } else if (current == null || !getFirstChild(parent)) {
          insertNode2(parent, value);
        } else replaceNode(parent, value, getFirstChild(parent));
      }
    }
    function normalize(value, multi, doNotUnwrap) {
      value = flatten(value, {
        skipNonRendered: true,
        doNotUnwrap
      });
      if (doNotUnwrap && typeof value === "function") return value;
      if (multi && !Array.isArray(value)) value = [value != null ? value : ""];
      if (Array.isArray(value)) {
        for (let i = 0, len = value.length; i < len; i++) {
          const item = value[i], t2 = typeof item;
          if (t2 === "string" || t2 === "number") value[i] = createTextNode2(item);
        }
      }
      return value;
    }
    function reconcileArrays(parentNode, a, b) {
      let bLength = b.length, aEnd = a.length, bEnd = bLength, aStart = 0, bStart = 0, after = getNextSibling(a[aEnd - 1]), map = null;
      const isLive = (n) => n && getParentNode(n) === parentNode;
      while (aStart < aEnd || bStart < bEnd) {
        if (a[aStart] === b[bStart] && isLive(a[aStart])) {
          aStart++;
          bStart++;
          continue;
        }
        while (a[aEnd - 1] === b[bEnd - 1] && isLive(a[aEnd - 1])) {
          aEnd--;
          bEnd--;
        }
        if (aEnd === aStart) {
          const node = bEnd < bLength ? bStart ? getNextSibling(b[bStart - 1]) : b[bEnd - bStart] : after;
          while (bStart < bEnd) insertNode2(parentNode, b[bStart++], node);
        } else if (bEnd === bStart) {
          while (aStart < aEnd) {
            if (!map || !map.has(a[aStart])) removeNode2(parentNode, a[aStart]);
            aStart++;
          }
        } else if (a[aStart] === b[bEnd - 1] && b[bStart] === a[aEnd - 1]) {
          const anchor = a[aStart];
          do {
            insertNode2(parentNode, a[--aEnd], anchor);
            bStart++;
            if (aStart >= aEnd - 1 || bStart >= bEnd) break;
          } while (a[aStart] === b[bEnd - 1] && b[bStart] === a[aEnd - 1]);
        } else {
          if (!map) {
            map = /* @__PURE__ */ new Map();
            let i = bStart;
            while (i < bEnd) map.set(b[i], i++);
          }
          const index = map.get(a[aStart]);
          if (index != null) {
            if (bStart < index && index < bEnd) {
              let i = aStart, sequence = 1, t2;
              while (++i < aEnd && i < bEnd) {
                if ((t2 = map.get(a[i])) == null || t2 !== index + sequence) break;
                sequence++;
              }
              if (sequence > index - bStart) {
                const node = a[aStart];
                while (bStart < index) insertNode2(parentNode, b[bStart++], node);
              } else replaceNode(parentNode, b[bStart++], a[aStart++]);
            } else aStart++;
          } else removeNode2(parentNode, a[aStart++]);
        }
      }
    }
    function cleanChildren(parent, current, marker, replacement) {
      if (marker === void 0) {
        let removed;
        while (removed = getFirstChild(parent)) removeNode2(parent, removed);
        replacement && insertNode2(parent, replacement);
        return "";
      }
      if (current.length) {
        let inserted = false;
        for (let i = current.length - 1; i >= 0; i--) {
          const el = current[i];
          if (replacement !== el) {
            const isParent = getParentNode(el) === parent;
            if (replacement && !inserted && !i) isParent ? replaceNode(parent, replacement, el) : insertNode2(parent, replacement, marker);
            else isParent && removeNode2(parent, el);
          } else inserted = true;
        }
      } else if (replacement) insertNode2(parent, replacement, marker);
    }
    function appendNodes(parent, array, marker) {
      for (let i = 0, len = array.length; i < len; i++) insertNode2(parent, array[i], marker);
    }
    function replaceNode(parent, newNode, oldNode) {
      insertNode2(parent, newNode, oldNode);
      removeNode2(parent, oldNode);
    }
    function collectNodes(value, nodes) {
      if (Array.isArray(value)) {
        for (let i = 0, len = value.length; i < len; i++) collectNodes(value[i], nodes);
      } else if (value != null && typeof value !== "string" && typeof value !== "number") {
        nodes.push(value);
      }
      return nodes;
    }
    function collectMounted(parent, value) {
      const nodes = collectNodes(value, []);
      if (!nodes.length && (typeof value === "string" || typeof value === "number")) {
        const node = getFirstChild(parent);
        if (node) nodes.push(node);
      }
      return nodes;
    }
    function defaultCleanupNodes(parent, nodes) {
      for (let i = 0, len = nodes.length; i < len; i++) {
        const node = nodes[i];
        if (getParentNode(node) === parent) removeNode2(parent, node);
      }
    }
    function spread2(node, props, skipChildren, options3) {
      const prevProps = {};
      const apply = (newProps) => {
        for (const prop in prevProps) {
          if (prop in newProps) continue;
          if (prop !== "ref") setProperty(node, prop, void 0, prevProps[prop]);
          delete prevProps[prop];
        }
        for (const prop in newProps) {
          const value = newProps[prop];
          if (value === prevProps[prop]) continue;
          if (prop === "ref") {
            (typeof value === "function" || Array.isArray(value)) && ref2(() => value, node);
          } else setProperty(node, prop, value, prevProps[prop]);
          prevProps[prop] = value;
        }
      };
      const childrenOptions = () => named(options3);
      if (Array.isArray(props)) {
        if (!skipChildren) insert2(node, () => {
          for (let i = props.length - 1; i >= 0; i--) {
            const s = resolveSource(props[i]);
            if (s != null && entryHas(s, "children")) return entryGet(s, "children");
          }
        }, void 0, void 0, childrenOptions());
        effect2(() => collectSources({}, props, void 0), apply, named(options3));
        return prevProps;
      }
      if (!skipChildren) {
        if (typeof props !== "function" && props != null && hasStaticKeys(props)) {
          const desc = Object.getOwnPropertyDescriptor(props, "children");
          if (desc !== void 0) {
            if (desc.get === void 0) insert2(node, desc.value, void 0, void 0, childrenOptions());
            else insert2(node, () => props.children, void 0, void 0, childrenOptions());
          }
        } else insert2(node, () => {
          const s = resolveSource(props);
          return s != null ? entryGet(s, "children") : void 0;
        }, void 0, void 0, childrenOptions());
      }
      effect2(() => {
        const s = resolveSource(props);
        const newProps = {};
        const table = resolvedTable(s);
        if (table !== void 0) {
          for (const [prop, leaf] of table) {
            if (typeof prop !== "string" || prop === "children") continue;
            newProps[prop] = leaf[prop];
          }
          return newProps;
        }
        if (s != null) {
          const view = viewOf(s);
          if (view instanceof OmitView) collectProps(newProps, view, SOURCE_OMIT);
          else if (view !== void 0) collectSources(newProps, view.sources, view.kinds);
          else collectProps(newProps, s, $PROXY in s ? SOURCE_PROXY : SOURCE_PLAIN);
        }
        return newProps;
      }, apply, named(options3));
      return prevProps;
    }
    function resolveSource(s) {
      return typeof s === "function" ? s() : s;
    }
    function entryHas(s, key) {
      const view = viewOf(s);
      return view instanceof OmitView ? sourceHas(view, SOURCE_OMIT, key) : key in s;
    }
    function entryGet(s, key) {
      const view = viewOf(s);
      return view instanceof OmitView ? sourceGet(view, SOURCE_OMIT, key) : s[key];
    }
    function collectSources(out, sources, kinds) {
      const resolved2 = [];
      const resolvedKinds = [];
      for (let i = 0; i < sources.length; i++) pushEntry(resolved2, resolvedKinds, sources[i], kinds !== void 0 ? kinds[i] : SOURCE_MEMO);
      for (let i = 0; i < resolved2.length; i++) collectProps(out, resolved2[i], resolvedKinds[i], resolved2, resolvedKinds, i + 1);
      return out;
    }
    function pushEntry(resolved2, kinds, s, kind) {
      if (kind !== SOURCE_MEMO) {
        resolved2.push(s);
        kinds.push(kind);
        return;
      }
      s = resolveSource(s);
      if (s == null) return;
      const view = viewOf(s);
      if (view instanceof OmitView) {
        resolved2.push(view);
        kinds.push(SOURCE_OMIT);
      } else if (view !== void 0) {
        const f = view.sources, k = view.kinds;
        for (let j = 0; j < f.length; j++) pushEntry(resolved2, kinds, f[j], k[j]);
      } else {
        resolved2.push(s);
        kinds.push($PROXY in s ? SOURCE_PROXY : SOURCE_PLAIN);
      }
    }
    function collectProps(out, s, kind, later, laterKinds, from) {
      const keys = sourceKeys(s, kind);
      outer: for (let i = 0; i < keys.length; i++) {
        const prop = keys[i];
        if (typeof prop !== "string" || prop === "children") continue;
        if (later !== void 0) {
          for (let j = from; j < later.length; j++) if (sourceHas(later[j], laterKinds[j], prop)) continue outer;
        }
        out[prop] = sourceGet(s, kind, prop);
      }
      return out;
    }
    function applyRef2(r, element) {
      Array.isArray(r) ? r.flat(Infinity).forEach((f) => f && f(element)) : r(element);
    }
    function ref2(fn2, element) {
      const resolved2 = untrack(fn2);
      runWithOwner(null, () => applyRef2(resolved2, element));
    }
    return {
      render(code, element) {
        let disposer, disposed = false, mounted = [];
        const cleanup2 = cleanupNodes || defaultCleanupNodes;
        try {
          createRoot2((dispose2) => {
            disposer = dispose2;
            const tree = code();
            const renderOptions = {
              schedule: true,
              onUpdate(value) {
                mounted = collectMounted(element, value);
              }
            };
            if (false) ;
            insert2(element, () => tree, void 0, void 0, renderOptions);
          });
          flush();
        } catch (err) {
          if (disposer) disposer();
          cleanup2(element, mounted);
          throw err;
        }
        return () => {
          if (disposed) return;
          disposed = true;
          disposer();
          cleanup2(element, mounted);
          mounted = [];
        };
      },
      insert: insert2,
      spread: spread2,
      createElement: createElement2,
      createTextNode: createTextNode2,
      insertNode: insertNode2,
      setProp(node, name, value, prev) {
        setProperty(node, name, value, prev);
        return value;
      },
      mergeProps: merge2,
      effect: effect2,
      memo,
      createComponent,
      applyRef: applyRef2,
      ref: ref2
    };
  }

  // node_modules/.pnpm/@uifw+solid-renderer@file+..+..+frameworks+solid-renderer/node_modules/@uifw/solid-renderer/src/colors.ts
  function parseColor(value) {
    if (typeof value === "string") {
      const hex = value.startsWith("#") ? value.slice(1) : value;
      if (hex.length !== 6 && hex.length !== 8) {
        throw new Error(`bad color ${value}`);
      }
      const r = Number.parseInt(hex.slice(0, 2), 16);
      const g = Number.parseInt(hex.slice(2, 4), 16);
      const b = Number.parseInt(hex.slice(4, 6), 16);
      const a = hex.length === 8 ? Number.parseInt(hex.slice(6, 8), 16) : 255;
      return {
        r,
        g,
        b,
        a
      };
    }
    if (value !== null && typeof value === "object" && "r" in value && "g" in value && "b" in value) {
      const color = value;
      return {
        r: color.r,
        g: color.g,
        b: color.b,
        a: color.a ?? 255
      };
    }
    throw new Error("color must be #RRGGBB, #RRGGBBAA, or {r,g,b,a}");
  }
  function colorBytes(value) {
    const color = parseColor(value);
    return [color.r, color.g, color.b, color.a];
  }

  // node_modules/.pnpm/@uifw+solid-renderer@file+..+..+frameworks+solid-renderer/node_modules/@uifw/solid-renderer/src/palette.ts
  var LIGHT = {
    bg: "#F3F6F4",
    background: "#F3F6F4",
    onBackground: "#14201A",
    surface: "#FFFFFF",
    onSurface: "#14201A",
    surfaceContainer: "#E5EEE8",
    onSurfaceContainer: "#14201A",
    text: "#14201A",
    textMuted: "#5C6B63",
    accent: "#127A32",
    primary: "#127A32",
    onPrimary: "#FFFFFF",
    primaryContainer: "#D7F5E3",
    onPrimaryContainer: "#06401C",
    danger: "#C83B3B",
    success: "#127A32",
    warning: "#A56A00",
    focusRing: "#127A32",
    scrim: "#00000080",
    outline: "#5C6B63",
    outlineVariant: "#D5E0D9",
    transparent: "#00000000"
  };
  var DARK = {
    bg: "#101412",
    background: "#101412",
    onBackground: "#F4F7F5",
    surface: "#1C2420",
    onSurface: "#F4F7F5",
    surfaceContainer: "#27312C",
    onSurfaceContainer: "#F4F7F5",
    text: "#F4F7F5",
    textMuted: "#9AA89F",
    accent: "#3DDC84",
    primary: "#3DDC84",
    onPrimary: "#052E16",
    primaryContainer: "#143524",
    onPrimaryContainer: "#C8F5D8",
    danger: "#FF6B6B",
    success: "#3DDC84",
    warning: "#E6B15A",
    focusRing: "#3DDC84",
    scrim: "#00000099",
    outline: "#6D7C74",
    outlineVariant: "#2C3832",
    transparent: "#00000000"
  };
  var HIGH_LIGHT = {
    bg: "#FFFFFF",
    background: "#FFFFFF",
    onBackground: "#000000",
    surface: "#FFFFFF",
    onSurface: "#000000",
    surfaceContainer: "#FFFFFF",
    onSurfaceContainer: "#000000",
    text: "#000000",
    textMuted: "#5C5C5C",
    accent: "#000000",
    primary: "#000000",
    onPrimary: "#FFFFFF",
    primaryContainer: "#FFFFFF",
    onPrimaryContainer: "#000000",
    danger: "#FF5A5A",
    success: "#000000",
    warning: "#000000",
    focusRing: "#000000",
    scrim: "#00000080",
    outline: "#000000",
    outlineVariant: "#000000",
    transparent: "#00000000"
  };
  var HIGH_DARK = {
    bg: "#000000",
    background: "#000000",
    onBackground: "#FFFFFF",
    surface: "#000000",
    onSurface: "#FFFFFF",
    surfaceContainer: "#000000",
    onSurfaceContainer: "#FFFFFF",
    text: "#FFFFFF",
    textMuted: "#A3A3A3",
    accent: "#FFFFFF",
    primary: "#FFFFFF",
    onPrimary: "#000000",
    primaryContainer: "#000000",
    onPrimaryContainer: "#FFFFFF",
    danger: "#FF5A5A",
    success: "#FFFFFF",
    warning: "#FFFFFF",
    focusRing: "#FFFFFF",
    scrim: "#00000080",
    outline: "#FFFFFF",
    outlineVariant: "#FFFFFF",
    transparent: "#00000000"
  };

  // node_modules/.pnpm/@uifw+solid-renderer@file+..+..+frameworks+solid-renderer/node_modules/@uifw/solid-renderer/src/defaults.ts
  var FONT_SIZE = 16;
  var TEXT_COLOR = LIGHT.text;
  var BUTTON_FILL = LIGHT.surface;
  var BUTTON_PRESSED = LIGHT.primary;
  var BUTTON_LABEL = LIGHT.onSurface;

  // node_modules/.pnpm/@uifw+solid-renderer@file+..+..+frameworks+solid-renderer/node_modules/@uifw/solid-renderer/src/layout-values.ts
  var PERCENT = /^(?:\d+(?:\.\d+)?|\.\d+)%$/;
  function parseAxisSize(value) {
    if (typeof value === "number") {
      if (!Number.isFinite(value) || value < 0) {
        throw new Error("bad axis");
      }
      return value;
    }
    if (value === "auto") {
      return "auto";
    }
    if (typeof value === "string" && PERCENT.test(value)) {
      const number = Number(value.slice(0, -1));
      if (!Number.isFinite(number) || number < 0) {
        throw new Error("bad axis");
      }
      return value;
    }
    throw new Error("bad axis");
  }
  function parseInsetEdge(value) {
    if (value === "auto") {
      return "auto";
    }
    if (typeof value === "number" && Number.isFinite(value)) {
      return value;
    }
    throw new Error("bad inset");
  }
  function integerIn(value, min, max) {
    if (typeof value !== "number" || !Number.isInteger(value) || value < min || value > max) {
      return void 0;
    }
    return value;
  }
  function parseGridLine(value) {
    if (value === "auto") {
      return "auto";
    }
    if (value !== null && typeof value === "object") {
      const record2 = value;
      const hasLine = Object.hasOwn(record2, "line");
      const hasSpan = Object.hasOwn(record2, "span");
      if (hasLine === hasSpan) {
        throw new Error("bad grid line");
      }
      if (hasLine) {
        const line = integerIn(record2.line, -32768, 32767);
        if (line === void 0 || line === 0) {
          throw new Error("bad grid line");
        }
        return {
          line
        };
      }
      const span = integerIn(record2.span, 1, 65535);
      if (span === void 0) {
        throw new Error("bad grid line");
      }
      return {
        span
      };
    }
    throw new Error("bad grid line");
  }

  // node_modules/.pnpm/@uifw+solid-renderer@file+..+..+frameworks+solid-renderer/node_modules/@uifw/solid-renderer/src/tree.ts
  function asNumber(value, fallback2) {
    return typeof value === "number" ? value : fallback2;
  }
  function axisCode(value) {
    if (value === "vertical") {
      return 1;
    }
    if (value === "horizontal") {
      return 2;
    }
    return 0;
  }
  function alignName(value) {
    if (value === "center" || value === "end") {
      return value;
    }
    return "start";
  }
  function crossName(value) {
    if (value === "start" || value === "center" || value === "end" || value === "baseline") {
      return value;
    }
    return "stretch";
  }
  function overflowName(value) {
    if (value === "ellipsis" || value === "visible") {
      return value;
    }
    return "clip";
  }
  function textAlignName(value) {
    if (value === "center" || value === "right") {
      return value;
    }
    return "left";
  }
  function justifyName(value) {
    if (value === "center" || value === "end" || value === "space-between" || value === "space-around" || value === "space-evenly") {
      return value;
    }
    return "start";
  }
  function trackList(value) {
    if (!Array.isArray(value)) {
      return [];
    }
    return value.map((item) => String(item));
  }
  function sizePair(value) {
    if (typeof value === "number") {
      return {
        w: value,
        h: value
      };
    }
    if (value !== null && typeof value === "object") {
      const record2 = value;
      return {
        w: asNumber(record2.w, 0),
        h: asNumber(record2.h, 0)
      };
    }
    return {
      w: 0,
      h: 0
    };
  }
  function hostRendererOptions(host2) {
    const nodes = /* @__PURE__ */ new Map();
    const transforms = /* @__PURE__ */ new Map();
    const paddings = /* @__PURE__ */ new Map();
    const sizes = /* @__PURE__ */ new Map();
    const insets = /* @__PURE__ */ new Map();
    const lines = /* @__PURE__ */ new Map();
    function forget(id, seen = /* @__PURE__ */ new Set()) {
      if (id === 0 || seen.has(id)) {
        return;
      }
      seen.add(id);
      const children2 = [];
      const listed = /* @__PURE__ */ new Set();
      let child = host2.firstChild(id);
      while (child !== 0 && !seen.has(child) && !listed.has(child)) {
        children2.push(child);
        listed.add(child);
        child = host2.nextSibling(child);
      }
      for (const kid of children2) {
        forget(kid, seen);
      }
      nodes.delete(id);
      transforms.delete(id);
      paddings.delete(id);
      sizes.delete(id);
      insets.delete(id);
      lines.delete(id);
    }
    function writeSize(id, patch) {
      const prev = sizes.get(id) ?? {
        w: 0,
        h: 0
      };
      const next = {
        ...prev,
        ...patch
      };
      sizes.set(id, next);
      if (typeof next.w === "number" && typeof next.h === "number") {
        host2.setSize(id, next.w, next.h);
        return;
      }
      host2.setAxisSize(id, next.w, next.h);
    }
    function writeInset(id, patch) {
      const prev = insets.get(id) ?? {
        top: "auto",
        right: "auto",
        bottom: "auto",
        left: "auto"
      };
      const next = {
        ...prev,
        ...patch
      };
      insets.set(id, next);
      host2.setInset(id, next.top, next.right, next.bottom, next.left);
    }
    function writeLines(id, patch) {
      const prev = lines.get(id) ?? {
        columnStart: "auto",
        columnEnd: "auto",
        rowStart: "auto",
        rowEnd: "auto"
      };
      const next = {
        ...prev,
        ...patch
      };
      lines.set(id, next);
      host2.setGridPlacement(id, next.columnStart, next.columnEnd, next.rowStart, next.rowEnd);
    }
    function writeTransform(id, patch) {
      const prev = transforms.get(id) ?? {
        x: 0,
        y: 0,
        rotate: 0,
        scale: 1
      };
      const next = {
        ...prev,
        ...patch
      };
      transforms.set(id, next);
      host2.setTransform(id, next.x, next.y, next.rotate, next.scale);
    }
    function writePadding(id, patch) {
      const prev = paddings.get(id) ?? {
        top: 0,
        right: 0,
        bottom: 0,
        left: 0
      };
      const next = {
        ...prev,
        ...patch
      };
      paddings.set(id, next);
      host2.setPaddingEdges(id, next.top, next.right, next.bottom, next.left);
    }
    function wrap(id) {
      let node = nodes.get(id);
      if (node === void 0) {
        node = {
          id
        };
        nodes.set(id, node);
      }
      return node;
    }
    function defined(id) {
      return id === 0 ? void 0 : wrap(id);
    }
    function setProperty(node, name, value) {
      switch (name) {
        case "gap":
          host2.setGap(node.id, asNumber(value, 0));
          return;
        case "edges": {
          const edge = asNumber(value, 0);
          host2.setPadding(node.id, edge);
          paddings.set(node.id, {
            top: edge,
            right: edge,
            bottom: edge,
            left: edge
          });
          return;
        }
        case "top":
          writePadding(node.id, {
            top: asNumber(value, 0)
          });
          return;
        case "right":
          writePadding(node.id, {
            right: asNumber(value, 0)
          });
          return;
        case "bottom":
          writePadding(node.id, {
            bottom: asNumber(value, 0)
          });
          return;
        case "left":
          writePadding(node.id, {
            left: asNumber(value, 0)
          });
          return;
        case "margin":
          if (typeof value === "number") {
            host2.setMargin(node.id, value, value, value, value);
          }
          return;
        case "min": {
          const size = sizePair(value);
          host2.setMinSize(node.id, size.w, size.h);
          return;
        }
        case "max": {
          const size = sizePair(value);
          host2.setMaxSize(node.id, size.w, size.h);
          return;
        }
        case "shrink":
          host2.setShrink(node.id, asNumber(value, 0));
          return;
        case "basis":
          host2.setBasis(node.id, asNumber(value, 0));
          return;
        case "alignSelf":
          host2.setAlignSelf(node.id, alignName(value));
          return;
        case "grow":
          host2.setGrow(node.id, asNumber(value, 0));
          return;
        case "radius":
          host2.setRadius(node.id, asNumber(value, 0));
          return;
        case "alpha":
          host2.setAlpha(node.id, asNumber(value, 255));
          return;
        case "align":
          host2.setAlign(node.id, alignName(value));
          return;
        case "maxLines":
          host2.setMaxLines(node.id, asNumber(value, 0));
          return;
        case "ellipsis":
          host2.setEllipsis(node.id, value == null ? "" : String(value));
          return;
        case "softWrap":
          host2.setSoftWrap(node.id, value !== false);
          return;
        case "overflow":
          host2.setOverflow(node.id, overflowName(value));
          return;
        case "textAlign":
          host2.setTextAlign(node.id, textAlignName(value));
          return;
        case "weight":
          host2.setFontWeight(node.id, asNumber(value, 400));
          return;
        case "direction":
          host2.setTextDirection(node.id, value === "rtl" ? "rtl" : "ltr");
          return;
        case "lineHeight":
          host2.setLineHeight(node.id, asNumber(value, 0));
          return;
        case "w":
          writeSize(node.id, {
            w: parseAxisSize(value)
          });
          return;
        case "h":
          writeSize(node.id, {
            h: parseAxisSize(value)
          });
          return;
        case "aspect":
          if (value === null) {
            host2.setAspectRatio(node.id, null);
            return;
          }
          if (typeof value === "number" && Number.isFinite(value) && value > 0) {
            host2.setAspectRatio(node.id, value);
            return;
          }
          throw new Error("bad aspect");
        case "absolute":
          host2.setPosition(node.id, value === true);
          return;
        case "insetTop":
          writeInset(node.id, {
            top: parseInsetEdge(value)
          });
          return;
        case "insetRight":
          writeInset(node.id, {
            right: parseInsetEdge(value)
          });
          return;
        case "insetBottom":
          writeInset(node.id, {
            bottom: parseInsetEdge(value)
          });
          return;
        case "insetLeft":
          writeInset(node.id, {
            left: parseInsetEdge(value)
          });
          return;
        case "columnStart":
          writeLines(node.id, {
            columnStart: parseGridLine(value)
          });
          return;
        case "columnEnd":
          writeLines(node.id, {
            columnEnd: parseGridLine(value)
          });
          return;
        case "rowStart":
          writeLines(node.id, {
            rowStart: parseGridLine(value)
          });
          return;
        case "rowEnd":
          writeLines(node.id, {
            rowEnd: parseGridLine(value)
          });
          return;
        case "family":
          host2.setFamily(node.id, value == null ? "" : String(value));
          return;
        case "justify":
          host2.setJustify(node.id, justifyName(value));
          return;
        case "cross":
          host2.setCross(node.id, crossName(value));
          return;
        case "wrap":
          host2.setWrap(node.id, value === true);
          return;
        case "fill":
          if (value === true) {
            host2.setFill(node.id);
          }
          return;
        case "axis":
          host2.setScrollAxis(node.id, axisCode(value));
          return;
        case "focusable":
          host2.setFocusable(node.id, value === true);
          return;
        case "x":
          writeTransform(node.id, {
            x: asNumber(value, 0)
          });
          return;
        case "y":
          writeTransform(node.id, {
            y: asNumber(value, 0)
          });
          return;
        case "rotate":
          writeTransform(node.id, {
            rotate: asNumber(value, 0)
          });
          return;
        case "scale":
          writeTransform(node.id, {
            scale: asNumber(value, 1)
          });
          return;
        case "fontSize":
          host2.setFontSize(node.id, asNumber(value, FONT_SIZE));
          return;
        case "color":
          host2.setColor(node.id, colorBytes(value));
          return;
        case "value":
          host2.setText(node.id, value == null ? "" : String(value));
          return;
        case "onClick":
          if (typeof value === "function") {
            host2.onClick(node.id, value);
          }
          return;
        case "onPress":
          if (typeof value === "function") {
            host2.onPress(node.id, value);
          }
          return;
        case "onRelease":
          if (typeof value === "function") {
            host2.onRelease(node.id, value);
          }
          return;
        case "onDragBegin":
          if (typeof value === "function") {
            host2.onDragBegin(node.id, value);
          }
          return;
        case "onDragBy":
          if (typeof value === "function") {
            host2.onDragBy(node.id, value);
          }
          return;
        case "onDragEnd":
          if (typeof value === "function") {
            host2.onDragEnd(node.id, value);
          }
          return;
        case "onFocus":
          if (typeof value === "function") {
            host2.onFocus(node.id, value);
          }
          return;
        case "onBlur":
          if (typeof value === "function") {
            host2.onBlur(node.id, value);
          }
          return;
        case "onKey":
          if (typeof value === "function") {
            host2.onKey(node.id, value);
          }
          return;
        case "onText":
          if (typeof value === "function") {
            host2.onText(node.id, value);
          }
          return;
        case "onTextReplaced":
          if (typeof value === "function") {
            host2.onTextReplaced(node.id, value);
          }
          return;
        case "onLink":
          if (typeof value === "function") {
            host2.onLink(node.id, value);
          }
          return;
        case "onComposing":
          if (typeof value === "function") {
            host2.onComposing(node.id, value);
          }
          return;
        case "onHover":
          if (typeof value === "function") {
            host2.onHover(node.id, value);
          }
          return;
        case "onLeave":
          if (typeof value === "function") {
            host2.onLeave(node.id, value);
          }
          return;
        case "onWheel":
          if (typeof value === "function") {
            host2.onWheel(node.id, value);
          }
          return;
        case "onScroll":
          if (typeof value === "function") {
            host2.onScroll(node.id, value);
          }
          return;
        case "onSecondaryClick":
          if (typeof value === "function") {
            host2.onSecondaryClick(node.id, value);
          }
          return;
        case "onZoom":
          if (typeof value === "function" && typeof host2.onZoom === "function") {
            host2.onZoom(node.id, value);
          }
          return;
        case "children":
        case "ref":
          return;
        default:
          throw new Error(`unknown prop ${name}`);
      }
    }
    return {
      createElement(tag, staticProps) {
        let id;
        switch (tag) {
          case "column":
            id = host2.column(asNumber(staticProps?.gap, 0));
            break;
          case "row":
            id = host2.row(asNumber(staticProps?.gap, 0));
            break;
          case "stack":
            id = host2.stack();
            break;
          case "align":
            id = host2.align(alignName(staticProps?.align));
            break;
          case "padding": {
            const edge = asNumber(staticProps?.edges, 0);
            id = host2.padding(edge);
            paddings.set(id, {
              top: edge,
              right: edge,
              bottom: edge,
              left: edge
            });
            break;
          }
          case "grid":
            id = host2.grid(trackList(staticProps?.columns), trackList(staticProps?.rows), asNumber(staticProps?.gap, 0));
            break;
          case "pointer":
            id = host2.pointerRegion();
            break;
          case "clip":
            id = host2.clip(asNumber(staticProps?.radius, 0));
            break;
          case "opacity":
            id = host2.opacity(asNumber(staticProps?.alpha, 255));
            break;
          case "scroll":
            id = host2.scroll(axisCode(staticProps?.axis));
            break;
          case "sized": {
            const width = staticProps?.w === void 0 ? 0 : parseAxisSize(staticProps.w);
            const height = staticProps?.h === void 0 ? 0 : parseAxisSize(staticProps.h);
            id = host2.sized(typeof width === "number" ? width : 0, typeof height === "number" ? height : 0);
            sizes.set(id, {
              w: width,
              h: height
            });
            if (typeof width !== "number" || typeof height !== "number") {
              host2.setAxisSize(id, width, height);
            }
            break;
          }
          case "transform":
            id = host2.transform();
            break;
          case "decorated":
            id = host2.decorated(colorBytes(staticProps?.color ?? BUTTON_FILL), asNumber(staticProps?.radius, 0));
            break;
          case "image":
            id = host2.image(staticProps?.source == null ? "" : String(staticProps.source), asNumber(staticProps?.w, 0), asNumber(staticProps?.h, 0), staticProps?.mode == null ? "fill" : String(staticProps.mode), staticProps?.required === true);
            break;
          default:
            throw new Error(`unknown tag ${tag}`);
        }
        const node = wrap(id);
        if (staticProps !== void 0) {
          for (const [name, value] of Object.entries(staticProps)) {
            if (name === "gap" || name === "edges" || name === "columns" || name === "rows" || name === "color" || name === "radius" || name === "alpha" || name === "source" || name === "w" || name === "h" || name === "mode" || name === "required" || name === "axis") {
              continue;
            }
            setProperty(node, name, value);
          }
        }
        return node;
      },
      createTextNode(value) {
        return wrap(host2.text(String(value), FONT_SIZE, colorBytes(TEXT_COLOR)));
      },
      replaceText(textNode, value) {
        host2.setText(textNode.id, String(value));
      },
      isTextNode(node) {
        return host2.isText(node.id);
      },
      setProperty(node, name, value) {
        setProperty(node, name, value);
      },
      insertNode(parent, node, anchor) {
        host2.insert(parent.id, node.id, anchor?.id ?? 0);
      },
      removeNode(parent, node) {
        forget(node.id);
        host2.remove(parent.id, node.id);
      },
      getParentNode(node) {
        return defined(host2.parent(node.id));
      },
      getFirstChild(node) {
        return defined(host2.firstChild(node.id));
      },
      getNextSibling(node) {
        return defined(host2.nextSibling(node.id));
      },
      retainedCounts() {
        return {
          nodes: nodes.size,
          transforms: transforms.size,
          paddings: paddings.size,
          sizes: sizes.size
        };
      },
      forget(id) {
        forget(id);
      }
    };
  }

  // node_modules/.pnpm/@uifw+solid-renderer@file+..+..+frameworks+solid-renderer/node_modules/@uifw/solid-renderer/runtime/host.ts
  var host = new Proxy({}, {
    get(_target, prop) {
      if (typeof prop !== "string") {
        return void 0;
      }
      const core = globalThis.core;
      const value = core?.[prop];
      if (typeof value === "function") {
        return value.bind(core);
      }
      return value;
    }
  });

  // node_modules/.pnpm/@uifw+solid-renderer@file+..+..+frameworks+solid-renderer/node_modules/@uifw/solid-renderer/runtime/text.ts
  function textContent(value) {
    if (typeof value === "function") {
      return textContent(value());
    }
    if (Array.isArray(value)) {
      return value.map((item) => textContent(item)).join("");
    }
    if (typeof value === "string") {
      return value;
    }
    if (typeof value === "number") {
      return String(value);
    }
    return "";
  }
  var options;
  function bindText(next) {
    options = next;
  }
  function Text(props) {
    const resolved2 = children(() => props.children);
    const node = options.createTextNode("");
    createRenderEffect2(() => textContent(resolved2()), (value) => {
      options.replaceText(node, value);
    });
    createRenderEffect2(() => props.color, (color) => {
      if (color !== void 0) {
        options.setProperty(node, "color", color);
      }
    });
    createTrackedEffect(() => {
      if (props.fontSize !== void 0) {
        options.setProperty(node, "fontSize", props.fontSize);
      }
      if (props.maxLines !== void 0) {
        options.setProperty(node, "maxLines", props.maxLines);
      }
      if (props.ellipsis !== void 0) {
        options.setProperty(node, "ellipsis", props.ellipsis);
      }
      if (props.softWrap !== void 0) {
        options.setProperty(node, "softWrap", props.softWrap);
      }
      if (props.overflow !== void 0) {
        options.setProperty(node, "overflow", props.overflow);
      }
      if (props.textAlign !== void 0) {
        options.setProperty(node, "textAlign", props.textAlign);
      }
      if (props.weight !== void 0) {
        options.setProperty(node, "weight", props.weight);
      }
      if (props.direction !== void 0) {
        options.setProperty(node, "direction", props.direction);
      }
      if (props.lineHeight !== void 0) {
        options.setProperty(node, "lineHeight", props.lineHeight);
      }
      if (props.family !== void 0) {
        options.setProperty(node, "family", props.family);
      }
    });
    return node;
  }

  // node_modules/.pnpm/@uifw+solid-renderer@file+..+..+frameworks+solid-renderer/node_modules/@uifw/solid-renderer/runtime/image.ts
  var options2;
  function bindImage(next) {
    options2 = next;
  }

  // node_modules/.pnpm/@uifw+solid-renderer@file+..+..+frameworks+solid-renderer/node_modules/@uifw/solid-renderer/runtime/frames.ts
  var ticks = /* @__PURE__ */ new Map();
  var nextTick = 1;
  function onFrame(fn2) {
    const id = nextTick;
    nextTick += 1;
    ticks.set(id, fn2);
    return () => {
      ticks.delete(id);
    };
  }
  function runFrames(targetMs, intervalMs) {
    for (const [id, fn2] of ticks) {
      try {
        fn2(targetMs, intervalMs);
      } catch (error) {
        ticks.delete(id);
        throw error;
      }
    }
  }
  var layouts = /* @__PURE__ */ new Map();
  function runLayouts() {
    for (const [id, fn2] of layouts) {
      try {
        fn2();
      } catch (error) {
        layouts.delete(id);
        throw error;
      }
    }
  }

  // node_modules/.pnpm/@uifw+solid-renderer@file+..+..+frameworks+solid-renderer/node_modules/@uifw/solid-renderer/runtime/slot.ts
  function bindSlot(deps) {
    return function createSlot2(initial, at, render2) {
      let root;
      let setItem;
      let setIndex;
      const dispose2 = runWithOwner(null, () => createRoot2((done) => {
        const [item, writeItem] = createSignal2(initial);
        const [index, writeIndex] = createSignal2(at);
        setItem = writeItem;
        setIndex = writeIndex;
        root = deps.createElement("column", {
          gap: 0
        });
        deps.insert(root, () => untrack(() => render2(item, index)));
        return done;
      }));
      return {
        get node() {
          return root.id;
        },
        set(next, index) {
          setItem(next);
          setIndex(index);
        },
        attach(parent, anchor) {
          deps.host.insert(parent, root.id, anchor ?? 0);
        },
        detach() {
          const parent = deps.host.parent(root.id);
          if (parent !== 0) {
            deps.host.detach(parent, root.id);
          }
        },
        place(parent, anchor = 0) {
          if (deps.host.parent(root.id) === parent && deps.host.nextSibling(root.id) === anchor) {
            return;
          }
          deps.host.insert(parent, root.id, anchor);
        },
        discard() {
          const parent = deps.host.parent(root.id);
          if (parent !== 0) {
            deps.host.detach(parent, root.id);
          }
          dispose2();
          deps.forget(root.id);
          deps.host.discard(root.id);
        }
      };
    };
  }

  // node_modules/.pnpm/@uifw+solid-renderer@file+..+..+frameworks+solid-renderer/node_modules/@uifw/solid-renderer/runtime/scroll-state.ts
  var listeners = /* @__PURE__ */ new Map();
  function phaseName(code) {
    if (code === 1) {
      return "dragging";
    }
    if (code === 2) {
      return "coasting";
    }
    if (code === 3) {
      return "settling";
    }
    return "idle";
  }
  function deliverScrollState(node, phase, vx, vy, ox, oy) {
    const sample = {
      phase: phaseName(phase),
      vx,
      vy,
      ox,
      oy
    };
    for (const fn2 of listeners.get(node) ?? []) {
      fn2(sample);
    }
  }
  function onScrollState(node, fn2) {
    const set = listeners.get(node) ?? /* @__PURE__ */ new Set();
    set.add(fn2);
    listeners.set(node, set);
    return () => {
      set.delete(fn2);
      if (set.size === 0) {
        listeners.delete(node);
      }
    };
  }

  // node_modules/.pnpm/@uifw+solid-renderer@file+..+..+frameworks+solid-renderer/node_modules/@uifw/solid-renderer/runtime/driver.ts
  var lists = [];
  var refreshes = [];
  function same(id, index, generation) {
    return id.index === index && id.generation === generation;
  }
  function deliverDriver(kind, index, generation, tag, i0, i1, f0, _f1, f2, _f3, _f4, f5, s0, s1) {
    if (kind === 2 && tag === 5) {
      for (const listener of refreshes) {
        if (same(listener.id, index, generation)) {
          listener.fn();
        }
      }
      return;
    }
    if (kind !== 1) {
      return;
    }
    for (const listener of lists) {
      if (!same(listener.id, index, generation)) {
        continue;
      }
      if (tag === 3) {
        listener.onEnd();
        continue;
      }
      if (tag === 2) {
        listener.onChrome({
          thumbOn: (i0 & 1) !== 0,
          thumbLen: f0,
          hThumbOn: (i0 & 2) !== 0,
          hThumbLen: f2,
          pinIndex: i1,
          pinKey: s0,
          extent: f5
        });
        continue;
      }
      if (tag !== 1) {
        continue;
      }
      if (i0 === 1) {
        listener.onOp({
          kind: "detach",
          key: s0
        });
      } else if (i0 === 2) {
        listener.onOp({
          kind: "adopt",
          key: s0,
          type: s1,
          index: i1
        });
      } else if (i0 === 3) {
        listener.onOp({
          kind: "set",
          key: s0,
          index: i1
        });
      } else if (i0 === 4) {
        listener.onOp({
          kind: "place",
          key: s0,
          before: s1
        });
      }
    }
  }
  function listenList(listener) {
    lists.push(listener);
    return () => {
      const index = lists.indexOf(listener);
      if (index >= 0) {
        lists.splice(index, 1);
      }
    };
  }
  function listenRefresh(id, fn2) {
    const listener = {
      id,
      fn: fn2
    };
    refreshes.push(listener);
    return () => {
      const index = refreshes.indexOf(listener);
      if (index >= 0) {
        refreshes.splice(index, 1);
      }
    };
  }

  // node_modules/.pnpm/@uifw+solid-renderer@file+..+..+frameworks+solid-renderer/node_modules/@uifw/solid-renderer/src/guard.ts
  function bag() {
    const core = globalThis.core;
    return core ?? {};
  }
  function fn(name) {
    const value = bag()[name];
    return typeof value === "function" ? value : null;
  }
  function callVoid(name, args) {
    const impl = fn(name);
    if (impl === null) {
      return;
    }
    impl.apply(bag(), args);
  }
  function scrollIntoView(id, edge) {
    callVoid("scrollIntoView", [id, edge]);
  }
  function animate(id, prop, from, to, stiffness, damping) {
    callVoid("animate", [id, prop, from, to, stiffness, damping]);
  }
  function setListenerErrorPolicy(policy) {
    callVoid("setListenerErrorPolicy", [policy]);
  }
  function setRecognizers(node, spec) {
    callVoid("setRecognizers", [node, spec]);
  }
  function setStroke(node, width, color) {
    callVoid("setStroke", [node, width, color]);
  }

  // node_modules/.pnpm/@uifw+solid-renderer@file+..+..+frameworks+solid-renderer/node_modules/@uifw/solid-renderer/src/direction.ts
  function directionFor(locale) {
    const language = locale.toLowerCase().split("-")[0] ?? "";
    if (language === "ar" || language === "he" || language === "fa" || language === "ur") {
      return "rtl";
    }
    return "ltr";
  }

  // node_modules/.pnpm/@uifw+solid-renderer@file+..+..+frameworks+solid-renderer/node_modules/@uifw/solid-renderer/runtime/back.ts
  var entries = [];
  var seq = 0;
  var hostId = 0;
  var installed = false;
  function onBack(priority, fn2) {
    if (typeof host.onBack !== "function") {
      return () => {
      };
    }
    if (!installed) {
      hostId = host.onBack(() => {
        const ordered = entries.slice().sort((left, right) => {
          if (left.priority !== right.priority) {
            return right.priority - left.priority;
          }
          return right.seq - left.seq;
        });
        for (const entry of ordered) {
          if (entry.fn()) {
            return true;
          }
        }
        return false;
      });
      installed = true;
    }
    const mine = ++seq;
    entries.push({
      priority,
      seq: mine,
      fn: fn2
    });
    return () => {
      entries = entries.filter((entry) => entry.seq !== mine);
      if (entries.length === 0 && installed && typeof host.offBack === "function") {
        host.offBack(hostId);
        installed = false;
        hostId = 0;
      }
    };
  }

  // node_modules/.pnpm/@uifw+solid-renderer@file+..+..+frameworks+solid-renderer/node_modules/@uifw/solid-renderer/runtime/focus.ts
  function focusHost() {
    return host;
  }
  function firstFocusable(id) {
    const core = focusHost();
    if (id !== 0 && core.isFocusable?.(id)) {
      return id;
    }
    let child = core.firstChild(id);
    while (child !== 0) {
      const found = firstFocusable(child);
      if (found !== 0) {
        return found;
      }
      child = core.nextSibling(child);
    }
    return 0;
  }
  function takeInitialFocus(scope, initialFocus) {
    const core = focusHost();
    if (typeof initialFocus === "number") {
      try {
        core.focus(initialFocus);
      } catch {
      }
      if (core.focused?.() === initialFocus) {
        return;
      }
    }
    const target = firstFocusable(scope);
    if (target === 0) {
      return;
    }
    try {
      core.focus(target);
    } catch {
    }
  }
  function restoreFocus(previous) {
    const core = focusHost();
    if (previous === 0) {
      core.blur?.();
      return;
    }
    try {
      core.focus(previous);
    } catch {
      core.blur?.();
      return;
    }
    if (core.focused?.() !== previous) {
      core.blur?.();
    }
  }

  // node_modules/.pnpm/@uifw+solid-renderer@file+..+..+frameworks+solid-renderer/node_modules/@uifw/solid-renderer/runtime/motion.ts
  var SPRING = {
    stiffness: 170,
    damping: 26
  };
  var pending = [];
  function flushMotion() {
    const batch = pending.splice(0);
    for (const done of batch) {
      done();
    }
  }
  function reduced() {
    if (typeof host.reduceMotion !== "function") {
      return false;
    }
    return host.reduceMotion();
  }
  function motionEnd(id, prop, fn2) {
    const core = host;
    if (typeof core.onMotionEnd !== "function") {
      return 0;
    }
    return core.onMotionEnd(id, prop, fn2);
  }
  function motionOff(id) {
    const core = host;
    if (typeof core.offMotionEnd === "function" && id !== 0) {
      core.offMotionEnd(id);
    }
  }
  function spring(options3) {
    const stiffness = options3.stiffness ?? SPRING.stiffness;
    const damping = options3.damping ?? SPRING.damping;
    const finish = () => options3.onDone?.();
    const linked = typeof host.animate === "function";
    if (reduced() || !linked) {
      options3.write?.(options3.to);
      if (options3.onDone) {
        pending.push(options3.onDone);
      }
      const stop = onFrame(() => {
        stop();
        flushMotion();
      });
      host.requestFrame();
      return;
    }
    animate(options3.id, options3.prop, options3.from, options3.to, stiffness, damping);
    let token = 0;
    token = motionEnd(options3.id, options3.prop, () => {
      motionOff(token);
      finish();
    });
  }

  // node_modules/.pnpm/@uifw+solid-renderer@file+..+..+frameworks+solid-renderer/node_modules/@uifw/solid-renderer/runtime/list-binding.ts
  function applyListOp(state, op, hooks2) {
    if (op.kind === "detach") {
      const slot2 = state.live.get(op.key);
      if (slot2 === void 0) {
        return;
      }
      state.live.delete(op.key);
      hooks2.note(op.key, 0);
      park(state, slot2, hooks2.poolSize());
      return;
    }
    if (op.kind === "adopt") {
      const item = hooks2.itemAt(op.index);
      let slot2 = state.live.get(op.key);
      if (slot2 === void 0 || slot2.type !== op.type) {
        if (slot2 !== void 0) {
          state.live.delete(op.key);
          hooks2.note(op.key, 0);
          park(state, slot2, hooks2.poolSize());
        }
        slot2 = take(state, op.type, item, op.index, op.key, hooks2.create);
        state.live.set(op.key, slot2);
      } else {
        slot2.set(item, op.index);
      }
      hooks2.note(op.key, slot2.node);
      return;
    }
    if (op.kind === "set") {
      const slot2 = state.live.get(op.key);
      if (slot2 === void 0) {
        return;
      }
      slot2.set(hooks2.itemAt(op.index), op.index);
      hooks2.note(op.key, slot2.node);
      return;
    }
    const slot = state.live.get(op.key);
    const parent = hooks2.parent();
    if (slot === void 0 || parent === 0) {
      return;
    }
    const before = op.before.length === 0 ? 0 : state.live.get(op.before)?.node ?? 0;
    slot.place(parent, before);
  }
  function park(state, slot, poolSize) {
    slot.detach();
    const pooled = state.pools.get(slot.type) ?? [];
    pooled.push(slot);
    state.pools.set(slot.type, pooled);
    while (pooled.length > poolSize) {
      pooled.shift()?.discard();
    }
  }
  function take(state, type, item, index, key, create) {
    const pooled = state.pools.get(type);
    const slot = pooled?.pop();
    if (slot === void 0) {
      return create(type, item, index, key);
    }
    slot.key = key;
    slot.set(item, index);
    return slot;
  }
  function bindListRenderer(driver, options3) {
    const state = {
      live: /* @__PURE__ */ new Map(),
      pools: /* @__PURE__ */ new Map()
    };
    const id = driver.id();
    return listenList({
      id,
      onOp(op) {
        applyListOp(state, op, {
          parent: options3.parent,
          poolSize: options3.poolSize,
          itemAt: options3.itemAt,
          note(key, node) {
            driver.noteRow(key, node);
          },
          create(type, item, index, key) {
            const slot = options3.createSlot(item, index, options3.render);
            return {
              key,
              type,
              get node() {
                return slot.node;
              },
              set(next, at) {
                slot.set(next, at);
              },
              detach() {
                slot.detach();
              },
              discard() {
                slot.discard();
              },
              place(parent, anchor) {
                slot.place(parent, anchor);
              }
            };
          }
        });
      },
      onChrome: options3.onChrome,
      onEnd: options3.onEnd
    });
  }

  // node_modules/.pnpm/@uifw+solid-renderer@file+..+..+frameworks+solid-renderer/node_modules/@uifw/solid-renderer/runtime/index.ts
  var scene = hostRendererOptions(host);
  bindText(scene);
  bindImage(scene);
  var renderer = createRenderer(scene);
  var {
    render,
    insert,
    spread,
    effect: effect3,
    memo: memo2,
    createComponent: createComponent2,
    setProp,
    mergeProps,
    applyRef,
    ref,
    createElement,
    createTextNode,
    insertNode
  } = renderer;
  var createSlot = bindSlot({
    createElement: scene.createElement,
    insert: (parent, accessor2) => {
      insert(parent, accessor2);
    },
    forget: (id) => {
      scene.forget(id);
    },
    host
  });
  function requestFrame() {
    host.requestFrame();
  }
  var overlayId = 0;
  function overlayParent() {
    return overlayId;
  }
  function removeNode(node) {
    const parent = scene.getParentNode(node);
    if (parent) {
      scene.removeNode(parent, node);
      return;
    }
    scene.forget(node.id);
    host.discard(node.id);
  }
  function boot(component) {
    const root = scene.createElement("stack", {
      fill: true
    });
    const content = scene.createElement("column", {
      gap: 0,
      fill: true
    });
    const overlay = scene.createElement("stack", {
      fill: true
    });
    scene.insertNode(root, content);
    scene.insertNode(root, overlay);
    overlayId = overlay.id;
    host.onFrame((targetMs, intervalMs) => {
      flush();
      runFrames(targetMs, intervalMs);
    });
    host.onLayout(() => {
      runLayouts();
      flush();
    });
    if (typeof host.onScrollState === "function") {
      host.onScrollState((node, phase, vx, vy, ox, oy) => {
        deliverScrollState(node, phase, vx, vy, ox, oy);
      });
    }
    if (typeof host.onDriver === "function") {
      host.onDriver((kind, index, generation, tag, i0, i1, f0, f1, f2, f3, f4, f5, s0, s1) => {
        deliverDriver(kind, index, generation, tag, i0, i1, f0, f1, f2, f3, f4, f5, s0, s1);
      });
    }
    render(component, content);
  }

  // node_modules/.pnpm/@uifw+solid-router@file+..+..+frameworks+solid-router/node_modules/@uifw/solid-router/src/path.ts
  function routePath(url) {
    const trimmed = url.trim();
    if (trimmed.length === 0) {
      return "/";
    }
    if (trimmed.includes("://")) {
      try {
        return new URL(trimmed).pathname || "/";
      } catch {
        return "/";
      }
    }
    return trimmed.startsWith("/") ? trimmed : `/${trimmed}`;
  }
  function matchPath(pattern, path) {
    const tokens = pattern.split("/").filter((part) => part.length > 0);
    const parts = routePath(path).split("/").filter((part) => part.length > 0);
    const params = {};
    let index = 0;
    for (; index < tokens.length; index += 1) {
      const token = tokens[index] ?? "";
      if (token === "*") {
        if (index !== tokens.length - 1) {
          return null;
        }
        params["*"] = parts.slice(index).join("/");
        return params;
      }
      if (token.startsWith(":")) {
        const value = parts[index];
        if (value === void 0) {
          return null;
        }
        params[token.slice(1)] = decodeURIComponent(value);
        continue;
      }
      if (token !== parts[index]) {
        return null;
      }
    }
    if (index !== parts.length) {
      return null;
    }
    return params;
  }

  // node_modules/.pnpm/@uifw+solid-router@file+..+..+frameworks+solid-router/node_modules/@uifw/solid-router/src/stack.ts
  var PageActiveContext = createContext2(() => true);
  var PageKeepContext = createContext2(() => false);
  function resolved(defs, path) {
    const pathname = routePath(path);
    for (const def of defs) {
      const params = matchPath(def.path, pathname);
      if (params) {
        return {
          path: pathname,
          pattern: def.path,
          params,
          keepAlive: !!def.keepAlive,
          component: def.component
        };
      }
    }
    return null;
  }
  var keySeq = 1;
  function nextKey() {
    const key = `route-${keySeq}`;
    keySeq += 1;
    return key;
  }
  function createRouter(defs, options3 = {}) {
    const initialPath = options3.initial ?? "/";
    const first = resolved(defs, initialPath);
    const [entries3, setEntries] = createSignal2(first ? [{
      ...first,
      key: nextKey(),
      phase: "live"
    }] : []);
    function push(path) {
      const match = resolved(defs, path);
      if (!match) {
        return false;
      }
      const list2 = entries3();
      const next = {
        ...match,
        key: nextKey(),
        phase: "enter"
      };
      const top = list2[list2.length - 1];
      if (!top) {
        setEntries([{
          ...next,
          phase: "live"
        }]);
        return true;
      }
      setEntries([...list2.slice(0, -1), {
        ...top,
        phase: "cover"
      }, next]);
      return true;
    }
    function replace(path) {
      const match = resolved(defs, path);
      if (!match) {
        return false;
      }
      const list2 = entries3();
      const top = list2[list2.length - 1];
      const next = {
        ...match,
        key: nextKey(),
        phase: "enter"
      };
      if (!top) {
        setEntries([{
          ...next,
          phase: "live"
        }]);
        return true;
      }
      setEntries([...list2.slice(0, -1), {
        ...top,
        phase: "exit"
      }, next]);
      return true;
    }
    function pop() {
      const list2 = entries3();
      if (list2.length <= 1) {
        return false;
      }
      const top = list2[list2.length - 1];
      if (!top) {
        return false;
      }
      if (top.phase === "exit") {
        return true;
      }
      setEntries(list2.map((entry, index) => {
        if (index === list2.length - 1) {
          return {
            ...entry,
            phase: "exit"
          };
        }
        if (entry.phase === "cover") {
          return {
            ...entry,
            phase: "live"
          };
        }
        return entry;
      }));
      return true;
    }
    function popTo(path) {
      const pathname = routePath(path);
      const list2 = entries3();
      let index = -1;
      for (let cursor = list2.length - 1; cursor >= 0; cursor -= 1) {
        if (list2[cursor]?.path === pathname && list2[cursor]?.phase !== "exit") {
          index = cursor;
          break;
        }
      }
      if (index < 0) {
        return false;
      }
      if (index === list2.length - 1) {
        return false;
      }
      setEntries(list2.map((entry, cursor) => cursor > index ? {
        ...entry,
        phase: "exit"
      } : entry));
      return true;
    }
    function reset2(path) {
      const match = resolved(defs, path);
      if (!match) {
        return false;
      }
      setEntries([{
        ...match,
        key: nextKey(),
        phase: "live"
      }]);
      return true;
    }
    function complete(key) {
      setEntries(entries3().flatMap((entry) => {
        if (entry.key !== key) {
          return [entry];
        }
        if (entry.phase === "exit") {
          return [];
        }
        if (entry.phase === "enter" || entry.phase === "cover") {
          return [{
            ...entry,
            phase: "live"
          }];
        }
        return [entry];
      }));
    }
    const router = {
      stack: entries3,
      depth: () => entries3().length,
      push,
      replace,
      pop,
      popTo,
      reset: reset2,
      complete
    };
    const releasePop = onBack(200, () => pop());
    const releaseApp = options3.onBack ? onBack(100, options3.onBack) : () => {
    };
    if (getOwner()) {
      onCleanup(() => {
        releasePop();
        releaseApp();
      });
    }
    const registry = globalThis.TurboModuleRegistry;
    const linking = registry?.get("Linking") ?? null;
    if (linking) {
      const initialUrl = linking.getInitialURL();
      if (initialUrl) {
        reset2(initialUrl);
      }
      linking.addEventListener("url", (event) => {
        push(event.url);
      });
    }
    return router;
  }

  // node_modules/.pnpm/@uifw+solid-router@file+..+..+frameworks+solid-router/node_modules/@uifw/solid-router/src/transition.ts
  var STACK_EXIT_ALPHA = Math.round(0.6 * 255);
  function travelDistance(measured, viewport) {
    if (measured >= 1) {
      return measured;
    }
    if (viewport >= 1) {
      return viewport;
    }
    return 360;
  }
  function stackMotion(phase, distance = 100) {
    const travel = Number.isFinite(distance) && distance > 0 ? distance : 100;
    if (phase === "enter") {
      return [{
        prop: "translateX",
        from: travel,
        to: 0
      }];
    }
    return [{
      prop: "translateX",
      from: 0,
      to: -Math.round(travel * 0.3)
    }, {
      prop: "opacity",
      from: 255,
      to: STACK_EXIT_ALPHA
    }];
  }

  // node_modules/.pnpm/@uifw+solid-router@file+..+..+frameworks+solid-router/node_modules/@uifw/solid-router/src/view.tsx
  function pageDistance(mover) {
    const measured = host.rect(mover).w;
    const width = host.metrics?.().width ?? 0;
    return travelDistance(measured, width);
  }
  function plainError(error, reset2) {
    const message = error instanceof Error ? error.message : String(error);
    var _el$ = createElement("column", {
      gap: 8
    }), _el$2 = createElement("pointer", {
      onClick: () => reset2()
    });
    insertNode(_el$, _el$2);
    insert(_el$, createComponent2(Text, {
      children: message
    }), _el$2);
    insert(_el$2, createComponent2(Text, {
      children: "retry"
    }));
    return _el$;
  }
  function unwrapError(error) {
    return typeof error === "function" ? error() : error;
  }
  function loggedError(error) {
    if (!(error instanceof Error)) {
      return String(error);
    }
    const stack = typeof error.stack === "string" ? error.stack : "";
    if (stack.length === 0) {
      return error.message;
    }
    return stack.includes(error.message) ? stack : `${error.message}
${stack}`;
  }
  function StackTransition(props) {
    let mover = 0;
    let fade = 0;
    let played = "";
    const [mounted, setMounted] = createSignal2(0);
    createTrackedEffect(() => {
      const phase = props.phase;
      if (mounted() === 0 || mover === 0 || fade === 0 || phase === played) {
        return;
      }
      played = phase;
      if (phase === "live") {
        host.setTransform(mover, 0, 0, 0, 1);
        host.setAlpha(fade, 255);
        return;
      }
      const steps = stackMotion(phase === "cover" ? "cover" : phase, pageDistance(mover));
      let left = steps.length;
      for (const step of steps) {
        if (step.prop === "opacity") {
          host.setAlpha(fade, Math.round(step.from));
        } else {
          host.setTransform(mover, step.from, 0, 0, 1);
        }
        spring({
          id: step.prop === "opacity" ? fade : mover,
          prop: step.prop,
          from: step.from,
          to: step.to,
          write: (value) => {
            if (step.prop === "opacity") {
              host.setAlpha(fade, Math.round(value));
            } else {
              host.setTransform(mover, value, 0, 0, 1);
            }
          },
          onDone: () => {
            left -= 1;
            if (left <= 0) {
              props.onDone();
            }
          }
        });
      }
    });
    var _el$3 = createElement("pointer", {
      fill: true
    });
    insert(_el$3, () => props.children);
    const body = _el$3;
    var _el$4 = createElement("opacity", {
      fill: true
    }), _el$5 = createElement("transform", {
      fill: true
    });
    insertNode(_el$4, _el$5);
    ref(() => (node) => {
      fade = node.id;
      setMounted(node.id);
    }, _el$4);
    ref(() => (node) => {
      mover = node.id;
    }, _el$5);
    insert(_el$5, (() => {
      var _c$ = memo2(() => props.background === void 0);
      return () => _c$() ? body : (() => {
        var _el$6 = createElement("decorated", {
          fill: true
        });
        insert(_el$6, body);
        effect3(() => props.background, (_v$, _$p) => {
          setProp(_el$6, "color", _v$, _$p);
        });
        return _el$6;
      })();
    })());
    return _el$4;
  }
  function RouteHost(props) {
    let previous = 0;
    const active = () => {
      const list2 = props.router.stack();
      for (let index = list2.length - 1; index >= 0; index -= 1) {
        const entry = list2[index];
        if (entry && entry.phase !== "exit" && entry.phase !== "cover") {
          return entry.key === props.entry().key;
        }
      }
      return false;
    };
    onCleanup(() => {
      try {
        host.popFocusScope();
      } catch {
      }
      restoreFocus(previous);
    });
    const Page = props.entry().component;
    const transition2 = props.transition ?? ((step) => createComponent2(StackTransition, {
      get phase() {
        return step.phase;
      },
      get onDone() {
        return step.onDone;
      },
      get background() {
        return props.background;
      },
      get children() {
        return step.children;
      }
    }));
    return createComponent2(PageActiveContext, {
      value: active,
      get children() {
        return createComponent2(PageKeepContext, {
          value: () => props.entry().keepAlive,
          get children() {
            var _el$7 = createElement("stack", {
              fill: true
            });
            ref(() => (node) => {
              previous = typeof host.focused === "function" ? host.focused() : 0;
              try {
                host.pushFocusScope(node.id);
              } catch {
              }
            }, _el$7);
            insert(_el$7, () => transition2({
              phase: props.entry().phase,
              onDone: () => props.router.complete(props.entry().key),
              children: createComponent2(Errored, {
                fallback: (err, reset2) => {
                  const error = unwrapError(err);
                  console.error(loggedError(error));
                  const fallback2 = props.errorFallback ?? plainError;
                  return fallback2(error, () => reset2());
                },
                get children() {
                  return createComponent2(Page, {
                    get params() {
                      return props.entry().params;
                    }
                  });
                }
              })
            }));
            return _el$7;
          }
        });
      }
    });
  }
  function RouterView(props) {
    var _el$8 = createElement("clip", {
      fill: true
    });
    insert(_el$8, createComponent2(For, {
      get each() {
        return props.router.stack();
      },
      keyed: (entry) => entry.key,
      children: (entry) => createComponent2(RouteHost, {
        get router() {
          return props.router;
        },
        entry,
        get transition() {
          return props.transition;
        },
        get background() {
          return props.background;
        },
        get errorFallback() {
          return props.errorFallback;
        }
      })
    }));
    return _el$8;
  }

  // node_modules/.pnpm/@uifw+solid-ui@file+..+..+frameworks+solid-ui/node_modules/@uifw/solid-ui/src/theme.ts
  var COEFF = {
    compact: 0.9,
    regular: 1,
    comfortable: 1.1
  };
  var DEFAULT_SNAPSHOT = {
    locale: "en",
    textScale: 1,
    brightness: "light",
    reduceMotion: false,
    boldText: false,
    highContrast: false
  };
  var port = null;
  var listening = false;
  var listenId = 0;
  var snapshot2 = DEFAULT_SNAPSHOT;
  var density = "regular";
  var observers = /* @__PURE__ */ new Set();
  function densityCoeff(value) {
    return COEFF[value];
  }
  function palette(brightness, highContrast) {
    if (highContrast && brightness === "dark") {
      return HIGH_DARK;
    }
    if (highContrast && brightness === "light") {
      return HIGH_LIGHT;
    }
    if (brightness === "dark") {
      return DARK;
    }
    return LIGHT;
  }
  function typeStyle(size, weight) {
    return {
      size,
      weight,
      lineHeight: size * 1.25
    };
  }
  function resolveTheme(settings, nextDensity) {
    const coeff = densityCoeff(nextDensity);
    return {
      locale: settings.locale,
      density: nextDensity,
      fontSize: {
        body: 16 * coeff,
        title: 20 * coeff
      },
      weight: settings.boldText ? {
        body: 600,
        emphasis: 700
      } : {
        body: 400,
        emphasis: 600
      },
      space: {
        s: 4 * coeff,
        m: 8 * coeff,
        l: 16 * coeff
      },
      radius: {
        s: 4 * coeff,
        m: 6 * coeff
      },
      shape: {
        none: 0,
        xs: 2 * coeff,
        s: 4 * coeff,
        m: 6 * coeff,
        l: 12 * coeff,
        full: 999
      },
      typography: {
        display: typeStyle(32 * coeff, settings.boldText ? 700 : 600),
        headline: typeStyle(24 * coeff, settings.boldText ? 700 : 600),
        title: typeStyle(20 * coeff, settings.boldText ? 700 : 600),
        body: typeStyle(16 * coeff, settings.boldText ? 600 : 400),
        label: typeStyle(14 * coeff, settings.boldText ? 600 : 500),
        caption: typeStyle(12 * coeff, settings.boldText ? 600 : 400)
      },
      color: palette(settings.brightness, settings.highContrast),
      reduceMotion: settings.reduceMotion,
      boldText: settings.boldText,
      highContrast: settings.highContrast,
      brightness: settings.brightness,
      textScale: settings.textScale
    };
  }
  function minTarget(size, nextDensity) {
    if (nextDensity === "comfortable") {
      return 48;
    }
    if (size === "compact" && nextDensity === "compact") {
      return 32;
    }
    return 44;
  }
  function notify() {
    for (const fn2 of observers) {
      fn2();
    }
  }
  function applySnapshot(next) {
    snapshot2 = next;
    notify();
  }
  function installThemePort(next) {
    port = next;
  }
  function onThemeChange(fn2) {
    observers.add(fn2);
    return () => {
      observers.delete(fn2);
    };
  }
  function ensureListening() {
    if (listening) {
      return;
    }
    if (port === null) {
      throw new Error("theme port is not installed");
    }
    snapshot2 = port.settings();
    listenId = port.onSettings(applySnapshot);
    listening = true;
  }
  function currentLocale() {
    return snapshot2.locale;
  }
  function createTheme(overrides) {
    ensureListening();
    return () => {
      const base = resolveTheme(snapshot2, density);
      if (overrides === void 0) {
        return base;
      }
      return {
        ...base,
        ...overrides
      };
    };
  }

  // node_modules/.pnpm/@uifw+solid-ui@file+..+..+frameworks+solid-ui/node_modules/@uifw/solid-ui/src/theme-bind.ts
  var settingsHost = host;
  installThemePort({
    settings: () => settingsHost.settings(),
    onSettings: (fn2) => settingsHost.onSettings(fn2),
    offSettings: (id) => settingsHost.offSettings(id)
  });

  // node_modules/.pnpm/@uifw+solid-ui@file+..+..+frameworks+solid-ui/node_modules/@uifw/solid-ui/src/colors.ts
  function colorBytes2(hex) {
    const h = hex.startsWith("#") ? hex.slice(1) : hex;
    if (h.length === 8) {
      return [Number.parseInt(h.slice(0, 2), 16), Number.parseInt(h.slice(2, 4), 16), Number.parseInt(h.slice(4, 6), 16), Number.parseInt(h.slice(6, 8), 16)];
    }
    return [Number.parseInt(h.slice(0, 2), 16), Number.parseInt(h.slice(2, 4), 16), Number.parseInt(h.slice(4, 6), 16), 255];
  }

  // node_modules/.pnpm/@uifw+solid-ui@file+..+..+frameworks+solid-ui/node_modules/@uifw/solid-ui/src/use-theme.ts
  function useTheme() {
    const read2 = createTheme();
    const [theme, setTheme] = createSignal2(read2());
    onCleanup(onThemeChange(() => setTheme(read2())));
    return theme;
  }

  // node_modules/.pnpm/@uifw+solid-ui@file+..+..+frameworks+solid-ui/node_modules/@uifw/solid-ui/src/focus.ts
  function bindFocusRing(id, focused) {
    const theme = useTheme();
    createTrackedEffect(() => {
      const node = id();
      if (node === 0) {
        return;
      }
      const on = focused();
      const color = on ? theme().color.focusRing : theme().color.transparent;
      setStroke(node, on ? 2 : 0, colorBytes2(color));
    });
  }

  // node_modules/.pnpm/@uifw+solid-ui@file+..+..+frameworks+solid-ui/node_modules/@uifw/solid-ui/src/semantics.ts
  var Role = {
    Generic: 0,
    Button: 1,
    Text: 2,
    Image: 3,
    Scroll: 4,
    Switch: 5,
    TextField: 6,
    SearchField: 7,
    TextArea: 8
  };

  // node_modules/.pnpm/@uifw+solid-ui@file+..+..+frameworks+solid-ui/node_modules/@uifw/solid-ui/src/Button.tsx
  function faded(hex, blocked2) {
    if (!blocked2) {
      return hex;
    }
    const body = hex.startsWith("#") ? hex.slice(1) : hex;
    return `#${body.slice(0, 6)}66`;
  }
  function Button(props) {
    const theme = useTheme();
    const [pressed, setPressed] = createSignal2(false);
    const [focused, setFocused] = createSignal2(false);
    const blocked2 = () => !!props.disabled || !!props.loading;
    const size = () => props.size ?? "regular";
    const variant = () => props.variant ?? "quiet";
    let node = 0;
    let box = 0;
    bindFocusRing(() => box, focused);
    const scale = () => theme().textScale;
    const padX = () => (size() === "compact" ? theme().space.m : theme().space.l) * scale();
    const padY = () => (size() === "compact" ? theme().space.s : theme().space.l) * scale();
    const target = () => minTarget(size(), theme().density) * scale();
    function face() {
      const color = pressed() ? variant() === "primary" ? theme().color.primaryContainer : theme().color.surfaceContainer : variant() === "primary" ? theme().color.accent : theme().color.surface;
      return faded(color, blocked2());
    }
    function labelColor() {
      const color = variant() === "primary" ? pressed() ? theme().color.onPrimaryContainer : theme().color.onPrimary : theme().color.text;
      return faded(color, blocked2());
    }
    function activate() {
      if (blocked2()) {
        return;
      }
      props.onClick();
    }
    function writeSemantics() {
      if (node === 0) {
        return;
      }
      host.setFocusable(node, !blocked2());
      host.setSemantics(node, Role.Button, props.label, void 0, void 0, void 0, void 0, blocked2() ? 1 : 0);
      const edge = target();
      host.setMinSize(node, edge, edge);
    }
    createTrackedEffect(() => {
      writeSemantics();
    });
    var _el$ = createElement("pointer", {
      onPress: () => {
        if (!blocked2()) {
          setPressed(true);
        }
      },
      onRelease: () => setPressed(false),
      onClick: activate,
      onFocus: () => setFocused(true),
      onBlur: () => setFocused(false),
      onKey: (key) => {
        if (key === "Enter") {
          activate();
        }
      },
      onText: (value) => {
        if (value === " ") {
          activate();
        }
      }
    }), _el$2 = createElement("decorated"), _el$3 = createElement("padding"), _el$4 = createElement("row", {
      justify: "center"
    });
    insertNode(_el$, _el$2);
    ref(() => (handle) => {
      node = handle.id;
      writeSemantics();
    }, _el$);
    insertNode(_el$2, _el$3);
    ref(() => (handle) => {
      box = handle.id;
    }, _el$2);
    insertNode(_el$3, _el$4);
    insert(_el$4, () => props.icon, null);
    insert(_el$4, createComponent2(Text, {
      get fontSize() {
        return theme().typography.label.size;
      },
      get color() {
        return labelColor();
      },
      get weight() {
        return theme().typography.label.weight;
      },
      get direction() {
        return directionFor(theme().locale);
      },
      get children() {
        return props.label;
      }
    }), null);
    effect3(() => ({
      e: props.wide ? "100%" : void 0,
      t: props.wide ? "100%" : void 0,
      a: face(),
      o: theme().shape.full,
      i: {
        h: target()
      },
      n: padX(),
      s: padX(),
      h: padY(),
      r: padY(),
      d: props.wide ? "100%" : void 0,
      l: theme().space.s,
      u: props.wide ? "100%" : void 0
    }), ({
      e,
      t: t2,
      a,
      o,
      i,
      n,
      s,
      h,
      r,
      d,
      l,
      u
    }, _p$) => {
      e !== _p$?.e && setProp(_el$, "w", e, _p$?.e);
      t2 !== _p$?.t && setProp(_el$2, "w", t2, _p$?.t);
      a !== _p$?.a && setProp(_el$2, "color", a, _p$?.a);
      o !== _p$?.o && setProp(_el$2, "radius", o, _p$?.o);
      i !== _p$?.i && setProp(_el$2, "min", i, _p$?.i);
      n !== _p$?.n && setProp(_el$3, "left", n, _p$?.n);
      s !== _p$?.s && setProp(_el$3, "right", s, _p$?.s);
      h !== _p$?.h && setProp(_el$3, "top", h, _p$?.h);
      r !== _p$?.r && setProp(_el$3, "bottom", r, _p$?.r);
      d !== _p$?.d && setProp(_el$3, "w", d, _p$?.d);
      l !== _p$?.l && setProp(_el$4, "gap", l, _p$?.l);
      u !== _p$?.u && setProp(_el$4, "w", u, _p$?.u);
    });
    return _el$;
  }

  // node_modules/.pnpm/@uifw+solid-ui@file+..+..+frameworks+solid-ui/node_modules/@uifw/solid-ui/src/host-ext.ts
  var coreHost = host;
  function nodeId(value) {
    if (typeof value === "number") {
      return value;
    }
    if (value !== null && typeof value === "object" && "id" in value) {
      const id = value.id;
      if (typeof id === "number") {
        return id;
      }
    }
    return 0;
  }

  // node_modules/.pnpm/@uifw+solid-ui@file+..+..+frameworks+solid-ui/node_modules/@uifw/solid-ui/src/overlay/registry.ts
  var nextId = 1;
  var entries2 = [];
  var releaseBack = null;
  function overlayEntries() {
    return entries2;
  }
  function registerOverlay(entry) {
    const spec = {
      id: nextId,
      modal: entry.modal,
      close: entry.close
    };
    nextId += 1;
    entries2.push(spec);
    if (releaseBack === null) {
      releaseBack = onBack(300, closeTopModal);
    }
    return () => {
      const index = entries2.findIndex((item) => item.id === spec.id);
      if (index >= 0) {
        entries2.splice(index, 1);
      }
    };
  }
  function closeTopModal() {
    for (let index = entries2.length - 1; index >= 0; index -= 1) {
      const entry = entries2[index];
      if (entry?.modal) {
        entry.close();
        return true;
      }
    }
    return false;
  }

  // node_modules/.pnpm/@uifw+solid-ui@file+..+..+frameworks+solid-ui/node_modules/@uifw/solid-ui/src/overlay.ts
  function moveChildren(from, to, skip = /* @__PURE__ */ new Set()) {
    const kids = [];
    let child = host.firstChild(from);
    while (child !== 0) {
      const next = host.nextSibling(child);
      if (!skip.has(child)) {
        kids.push(child);
      }
      child = next;
    }
    for (const id of kids) {
      host.insert(to, id, 0);
    }
  }

  // node_modules/.pnpm/@uifw+solid-ui@file+..+..+frameworks+solid-ui/node_modules/@uifw/solid-ui/src/Dialog.tsx
  function Dialog(props) {
    const theme = useTheme();
    let placeholder = 0;
    let layer = null;
    let closing = false;
    let epoch = 0;
    function closeLayer() {
      if (layer === null) {
        return;
      }
      const current = layer;
      layer = null;
      closing = false;
      try {
        moveChildren(current.slot, placeholder);
        if (current.focused) {
          host.popFocusScope();
          restoreFocus(current.previous);
        }
      } finally {
        current.unregister?.();
        removeNode({
          id: current.fade
        });
      }
    }
    function beginClose() {
      if (layer === null || closing) {
        return;
      }
      closing = true;
      layer.unregister?.();
      layer.unregister = null;
      const fade = layer.fade;
      const token = ++epoch;
      spring({
        id: fade,
        prop: "opacity",
        from: 255,
        to: 0,
        write: (value) => host.setAlpha(fade, Math.round(value)),
        onDone: () => {
          if (token === epoch) {
            closeLayer();
          }
        }
      });
    }
    function ensureLayer() {
      if (layer !== null) {
        return layer;
      }
      if (placeholder === 0) {
        return null;
      }
      const overlay = nodeId(overlayParent());
      const fade = host.opacity(0);
      host.setFill(fade);
      host.insert(overlay, fade, 0);
      const shell = host.stack();
      host.setFill(shell);
      host.insert(fade, shell, 0);
      const scrim = host.pointerRegion();
      host.setFill(scrim);
      host.onClick(scrim, () => props.onClose());
      host.insert(shell, scrim, 0);
      const paint = host.decorated(colorBytes2(theme().color.scrim), 0);
      host.setFill(paint);
      host.insert(scrim, paint, 0);
      const frame = host.align("center");
      host.setFill(frame);
      host.insert(shell, frame, 0);
      const content = host.column(theme().space.m);
      host.insert(frame, content, 0);
      const title = props.title ?? "";
      let titleId = 0;
      if (title.length > 0) {
        titleId = host.text(title, theme().fontSize.title, colorBytes2(theme().color.text));
        host.setFontWeight(titleId, theme().weight.emphasis);
        host.setTextDirection(titleId, directionFor(theme().locale));
        host.insert(content, titleId, 0);
      }
      host.setSemantics(shell, Role.Generic, title.length > 0 ? title : void 0);
      const slot = host.column(theme().space.s);
      host.insert(content, slot, 0);
      layer = {
        shell,
        slot,
        titleId,
        fade,
        previous: 0,
        back: 0,
        focused: false,
        unregister: null
      };
      spring({
        id: fade,
        prop: "opacity",
        from: 0,
        to: 255,
        write: (value) => host.setAlpha(fade, Math.round(value))
      });
      return layer;
    }
    function sync() {
      if (placeholder === 0) {
        return;
      }
      if (!props.open) {
        beginClose();
        return;
      }
      if (closing) {
        epoch += 1;
        closing = false;
      }
      const current = ensureLayer();
      if (current === null) {
        return;
      }
      if (current.titleId !== 0) {
        host.setTextDirection(current.titleId, directionFor(theme().locale));
      }
      if (host.firstChild(placeholder) !== 0) {
        moveChildren(placeholder, current.slot);
      }
      if (current.focused) {
        return;
      }
      try {
        const previous = coreHost.focused();
        host.pushFocusScope(current.shell);
        current.previous = previous;
        current.unregister = registerOverlay({
          modal: true,
          close: () => props.onClose()
        });
        current.focused = true;
        takeInitialFocus(current.shell, props.initialFocus);
      } catch {
      }
    }
    const stop = onFrame(() => {
      sync();
    });
    onCleanup(() => {
      stop();
      closeLayer();
    });
    var _el$ = createElement("clip", {
      min: {
        w: 0,
        h: 0
      },
      max: {
        w: 0,
        h: 0
      }
    });
    ref(() => (node) => {
      placeholder = node.id;
      host.setMinSize(node.id, 0, 0);
      host.setMaxSize(node.id, 0, 0);
    }, _el$);
    insert(_el$, () => props.children);
    return _el$;
  }

  // node_modules/.pnpm/@uifw+solid-ui@file+..+..+frameworks+solid-ui/node_modules/@uifw/solid-ui/src/i18n.ts
  var ZH = {
    switchOn: "\u5F00",
    switchOff: "\u5173",
    retry: "\u91CD\u8BD5",
    back: "\u8FD4\u56DE",
    errorTitle: "\u51FA\u9519\u4E86"
  };
  var EN = {
    switchOn: "On",
    switchOff: "Off",
    retry: "Retry",
    back: "Back",
    errorTitle: "Something went wrong"
  };
  function messagesFor(locale) {
    return locale.startsWith("zh") ? ZH : EN;
  }
  function t(key, params, locale = currentLocale()) {
    const template = messagesFor(locale)[key];
    if (!params) {
      return template;
    }
    return template.replace(/\{(\w+)\}/g, (_match, name) => {
      const value = params[name];
      return value === void 0 ? "" : String(value);
    });
  }

  // node_modules/.pnpm/@uifw+solid-ui@file+..+..+frameworks+solid-ui/node_modules/@uifw/solid-ui/src/errors.ts
  setListenerErrorPolicy("report");
  function messageOf(error) {
    if (error instanceof Error) {
      return error.message;
    }
    return String(error);
  }
  function stackOf(error) {
    if (error instanceof Error && typeof error.stack === "string") {
      return error.stack;
    }
    return "";
  }

  // node_modules/.pnpm/@uifw+solid-ui@file+..+..+frameworks+solid-ui/node_modules/@uifw/solid-ui/src/errors.tsx
  function ErrorPage(props) {
    const theme = useTheme();
    var _el$ = createElement("column", {
      w: "100%"
    });
    insert(_el$, createComponent2(Text, {
      get fontSize() {
        return theme().typography.title.size;
      },
      get color() {
        return theme().color.danger;
      },
      get weight() {
        return theme().typography.title.weight;
      },
      get direction() {
        return directionFor(theme().locale);
      },
      get children() {
        return props.message;
      }
    }), null);
    insert(_el$, (() => {
      var _c$ = memo2(() => !!props.stack);
      return () => _c$() ? createComponent2(Text, {
        get fontSize() {
          return theme().typography.caption.size;
        },
        get color() {
          return theme().color.textMuted;
        },
        get direction() {
          return directionFor(theme().locale);
        },
        get children() {
          return props.stack;
        }
      }) : null;
    })(), null);
    insert(_el$, createComponent2(Button, {
      wide: true,
      variant: "primary",
      get label() {
        return t("retry");
      },
      onClick: () => props.retry()
    }), null);
    insert(_el$, (() => {
      var _c$2 = memo2(() => !!props.onBack);
      return () => _c$2() ? createComponent2(Button, {
        get label() {
          return t("back");
        },
        onClick: () => props.onBack?.()
      }) : null;
    })(), null);
    effect3(() => theme().space.l, (_v$, _$p) => {
      setProp(_el$, "gap", _v$, _$p);
    });
    return _el$;
  }

  // node_modules/.pnpm/@uifw+solid-ui@file+..+..+frameworks+solid-ui/node_modules/@uifw/solid-ui/src/field/caret.tsx
  var HIDDEN = "transparent";
  function CaretLayer(props) {
    var _el$ = createElement("stack"), _el$2 = createElement("transform"), _el$3 = createElement("decorated"), _el$4 = createElement("sized", {
      w: 2
    });
    insertNode(_el$, _el$2);
    insert(_el$, () => props.rects().map((rect) => (() => {
      var _el$5 = createElement("transform"), _el$6 = createElement("decorated"), _el$7 = createElement("sized");
      insertNode(_el$5, _el$6);
      insertNode(_el$6, _el$7);
      effect3(() => ({
        e: rect.x,
        t: rect.y,
        a: `${props.accent()}66`,
        o: rect.w,
        i: rect.h
      }), ({
        e,
        t: t2,
        a,
        o,
        i
      }, _p$) => {
        e !== _p$?.e && setProp(_el$5, "x", e, _p$?.e);
        t2 !== _p$?.t && setProp(_el$5, "y", t2, _p$?.t);
        a !== _p$?.a && setProp(_el$6, "color", a, _p$?.a);
        o !== _p$?.o && setProp(_el$7, "w", o, _p$?.o);
        i !== _p$?.i && setProp(_el$7, "h", i, _p$?.i);
      });
      return _el$5;
    })()), _el$2);
    insertNode(_el$2, _el$3);
    insertNode(_el$3, _el$4);
    effect3(() => ({
      e: props.caretX(),
      t: props.caretY(),
      a: props.showCaret() ? props.accent() : props.transparent() || HIDDEN,
      o: props.caretH()
    }), ({
      e,
      t: t2,
      a,
      o
    }, _p$) => {
      e !== _p$?.e && setProp(_el$2, "x", e, _p$?.e);
      t2 !== _p$?.t && setProp(_el$2, "y", t2, _p$?.t);
      a !== _p$?.a && setProp(_el$3, "color", a, _p$?.a);
      o !== _p$?.o && setProp(_el$4, "h", o, _p$?.o);
    });
    return _el$;
  }

  // node_modules/.pnpm/@uifw+solid-ui@file+..+..+frameworks+solid-ui/node_modules/@uifw/solid-ui/src/field/keyboard.ts
  var EDIT_LOAD = 0;
  var EDIT_LEFT = 1;
  var EDIT_RIGHT = 2;
  var EDIT_BACKSPACE = 3;
  var EDIT_INSERT = 4;
  var EDIT_COMPOSING = 5;
  var EDIT_UNDO = 7;
  var EDIT_REDO = 8;
  var EDIT_SELECT_ALL = 9;
  var EDIT_COPY = 10;
  var EDIT_CUT = 11;
  var EDIT_CARET = 12;
  var EDIT_FORWARD_DELETE = 13;
  var EDIT_NEWLINE = 14;
  var EDIT_WORD_LEFT = 15;
  var EDIT_WORD_RIGHT = 16;
  var EDIT_LINE_START = 17;
  var EDIT_LINE_END = 18;
  var EDIT_EXTEND_LEFT = 19;
  var EDIT_EXTEND_RIGHT = 20;
  var EDIT_EXTEND_WORD_LEFT = 21;
  var EDIT_EXTEND_WORD_RIGHT = 22;
  var EDIT_EXTEND_LINE_START = 23;
  var EDIT_EXTEND_LINE_END = 24;
  var EDIT_UP = 25;
  var EDIT_DOWN = 26;
  var EDIT_EXTEND_UP = 27;
  var EDIT_EXTEND_DOWN = 28;
  var EDIT_EXTEND_CARET = 29;
  var EDIT_DELETE_WORD = 30;
  var EDIT_DELETE_WORD_FORWARD = 31;
  var EDIT_DELETE_TO_LINE_START = 32;
  var EDIT_DELETE_TO_LINE_END = 33;
  var EDIT_DOCUMENT_START = 34;
  var EDIT_DOCUMENT_END = 35;
  var EDIT_EXTEND_DOCUMENT_START = 36;
  var EDIT_EXTEND_DOCUMENT_END = 37;
  var EDIT_COMMIT = 38;
  var KEYBOARD = {
    text: 0,
    multiline: 1,
    number: 2,
    phone: 3,
    email: 4,
    url: 5,
    name: 6,
    password: 7
  };
  var ACTION = {
    none: 0,
    done: 1,
    next: 2,
    search: 3,
    go: 4,
    send: 5,
    previous: 6,
    newline: 7
  };
  var SUBMIT_ACTIONS = /* @__PURE__ */ new Set(["done", "search", "go", "send", "next"]);
  function imeConfig(keyboard, action2, multiline) {
    const keyboardCode = keyboard && keyboard !== "text" ? KEYBOARD[keyboard] ?? 0 : multiline ? 1 : 0;
    let actionCode;
    if (action2) {
      actionCode = ACTION[action2] ?? 0;
    } else {
      actionCode = multiline ? ACTION.newline : ACTION.done;
    }
    return {
      keyboard: keyboardCode ?? 0,
      action: actionCode ?? 0
    };
  }
  function enterInsertsNewline(multiline, action2) {
    if (!multiline) {
      return false;
    }
    if (action2 !== void 0 && SUBMIT_ACTIONS.has(action2)) {
      return false;
    }
    return true;
  }

  // node_modules/.pnpm/@uifw+solid-ui@file+..+..+frameworks+solid-ui/node_modules/@uifw/solid-ui/src/field/controller.ts
  function utf16Len(value) {
    let count = 0;
    for (const char of value) {
      count += (char.codePointAt(0) ?? 0) > 65535 ? 2 : 1;
    }
    return count;
  }
  function createFieldController(options3) {
    const [text, setText] = createSignal2("");
    const [composing, setComposing] = createSignal2(null);
    const [composeStart, setComposeStart] = createSignal2(-1);
    const [composeEnd, setComposeEnd] = createSignal2(-1);
    const [start, setStart] = createSignal2(0);
    const [end, setEnd] = createSignal2(0);
    const [validError, setValidError] = createSignal2("");
    let mask = null;
    let transform = null;
    const selection = () => ({
      start: start(),
      end: end()
    });
    function formatted(value) {
      let next = value;
      if (mask) {
        next = mask.format(next);
      }
      if (transform) {
        next = transform(next);
      }
      return next;
    }
    function usesFormat() {
      return mask !== null || transform !== null;
    }
    function publishChange(before, next) {
      if (next === before) {
        return;
      }
      options3.onChange?.(next);
      if (options3.validate) {
        setValidError(options3.validate(next) ?? "");
      } else {
        setValidError("");
      }
    }
    function apply(op, extra, kind) {
      if ((kind === "change" || kind === "compose") && options3.allowChange && !options3.allowChange()) {
        return;
      }
      const before = text();
      const next = options3.textEdit(op, extra);
      const limit = options3.maxLength?.();
      if (kind !== "load" && limit !== void 0 && utf16Len(next.text) > limit) {
        const reverted = options3.textEdit(EDIT_LOAD, before);
        setText(reverted.text);
        setStart(reverted.start);
        setEnd(reverted.end);
        return;
      }
      setText(next.text);
      setStart(next.start);
      setEnd(next.end);
      if (kind === "change") {
        publishChange(before, next.text);
      }
    }
    function commitFormatted(raw) {
      const next = formatted(raw);
      apply(EDIT_COMMIT, next, "change");
    }
    function rawBase() {
      const full = text();
      const extra = composing();
      if (extra === null || extra.length === 0) {
        return full;
      }
      if (full.endsWith(extra)) {
        return full.slice(0, full.length - extra.length);
      }
      return full;
    }
    const controller = {
      text,
      selection,
      composing,
      composeStart,
      composeEnd,
      error: validError,
      select(from, to) {
        apply(EDIT_CARET, String(from), "motion");
        apply(EDIT_EXTEND_CARET, String(to), "motion");
      },
      selectAll() {
        apply(EDIT_SELECT_ALL, "", "motion");
      },
      insert(value) {
        if (usesFormat()) {
          const base = rawBase();
          commitFormatted(base + value);
          setComposing(null);
          return;
        }
        apply(EDIT_INSERT, value, "change");
      },
      undo() {
        apply(EDIT_UNDO, "", "change");
      },
      redo() {
        apply(EDIT_REDO, "", "change");
      },
      clear() {
        setComposing(null);
        if (usesFormat()) {
          apply(EDIT_COMMIT, formatted(""), "change");
          return;
        }
        apply(EDIT_SELECT_ALL, "", "motion");
        apply(EDIT_INSERT, "", "change");
      },
      setMask(next) {
        mask = next;
        if (composing() !== null) {
          return;
        }
        if (next === null && transform === null) {
          return;
        }
        commitFormatted(text());
      },
      setTransform(next) {
        transform = next;
        if (composing() !== null) {
          return;
        }
        if (next === null && mask === null) {
          return;
        }
        commitFormatted(text());
      },
      focus() {
        options3.focus?.();
      },
      onComposing(value) {
        if (value === null || value.length === 0) {
          setComposing(null);
          setComposeStart(-1);
          setComposeEnd(-1);
          apply(EDIT_COMPOSING, "", "change");
          return;
        }
        setComposing(value);
        apply(EDIT_COMPOSING, value, "compose");
        const caret = end();
        const units = utf16Len(value);
        setComposeStart(Math.max(0, caret - units));
        setComposeEnd(caret);
      },
      onText(value) {
        const raw = rawBase() + value;
        setComposing(null);
        if (usesFormat()) {
          commitFormatted(raw);
          return;
        }
        apply(EDIT_INSERT, value, "change");
      },
      onTextReplaced(value) {
        setComposing(null);
        if (usesFormat()) {
          commitFormatted(value);
          return;
        }
        setText(value);
        const caret = utf16Len(value);
        setStart(caret);
        setEnd(caret);
        options3.onChange?.(value);
      },
      load(value) {
        if (value === text()) {
          return;
        }
        if (usesFormat()) {
          commitFormatted(value);
          return;
        }
        apply(EDIT_LOAD, value, "load");
      },
      move(collapsed, extend, shift) {
        apply(shift ? extend : collapsed, "", "motion");
      },
      apply,
      caretAt(unit) {
        apply(EDIT_CARET, String(unit), "motion");
      },
      replace(next, anchor, caret, composeFrom, composeTo) {
        const before = text();
        const limit = utf16Len(next);
        const marked = composeFrom >= 0 && composeTo > composeFrom && composeTo <= limit ? next.slice(composeFrom, composeTo) : "";
        if (marked.length > 0) {
          setComposing(marked);
          setComposeStart(composeFrom);
          setComposeEnd(composeTo);
        } else {
          setComposing(null);
          setComposeStart(-1);
          setComposeEnd(-1);
        }
        if (next !== before) {
          apply(EDIT_LOAD, next, "load");
          options3.onChange?.(next);
        }
        const len = utf16Len(text());
        const from = Math.max(0, Math.min(anchor, len));
        const to = Math.max(from, Math.min(Math.max(anchor, caret), len));
        if (start() !== from || end() !== to) {
          apply(EDIT_CARET, String(from), "motion");
          if (to !== from) {
            apply(EDIT_EXTEND_CARET, String(to), "motion");
          }
        }
      },
      extendCaret(unit) {
        apply(EDIT_EXTEND_CARET, String(unit), "motion");
      },
      handleKey(key, multiline, password, submitEnter, onSubmit) {
        const shift = key.startsWith("Shift+");
        const name = shift ? key.slice("Shift+".length) : key;
        if (name === "Left") {
          controller.move(EDIT_LEFT, EDIT_EXTEND_LEFT, shift);
          return;
        }
        if (name === "Right") {
          controller.move(EDIT_RIGHT, EDIT_EXTEND_RIGHT, shift);
          return;
        }
        if (name === "Up") {
          if (multiline) {
            controller.move(EDIT_UP, EDIT_EXTEND_UP, shift);
          } else {
            options3.onVertical?.("up");
          }
          return;
        }
        if (name === "Down") {
          if (multiline) {
            controller.move(EDIT_DOWN, EDIT_EXTEND_DOWN, shift);
          } else {
            options3.onVertical?.("down");
          }
          return;
        }
        if (name === "Home") {
          controller.move(EDIT_LINE_START, EDIT_EXTEND_LINE_START, shift);
          return;
        }
        if (name === "End") {
          controller.move(EDIT_LINE_END, EDIT_EXTEND_LINE_END, shift);
          return;
        }
        if (name === "WordLeft") {
          controller.move(EDIT_WORD_LEFT, EDIT_EXTEND_WORD_LEFT, shift);
          return;
        }
        if (name === "WordRight") {
          controller.move(EDIT_WORD_RIGHT, EDIT_EXTEND_WORD_RIGHT, shift);
          return;
        }
        if (name === "DocumentStart") {
          controller.move(EDIT_DOCUMENT_START, EDIT_EXTEND_DOCUMENT_START, shift);
          return;
        }
        if (name === "DocumentEnd") {
          controller.move(EDIT_DOCUMENT_END, EDIT_EXTEND_DOCUMENT_END, shift);
          return;
        }
        if (name === "SelectAll") {
          controller.selectAll();
          return;
        }
        if (name === "Copy") {
          if (!password) {
            apply(EDIT_COPY, "", "motion");
          }
          return;
        }
        if (name === "Cut") {
          if (!password) {
            apply(EDIT_CUT, "", "change");
          }
          return;
        }
        if (name === "Backspace") {
          apply(EDIT_BACKSPACE, "", "change");
          return;
        }
        if (name === "Delete") {
          apply(EDIT_FORWARD_DELETE, "", "change");
          return;
        }
        if (name === "DeleteWord") {
          apply(EDIT_DELETE_WORD, "", "change");
          return;
        }
        if (name === "DeleteWordForward") {
          apply(EDIT_DELETE_WORD_FORWARD, "", "change");
          return;
        }
        if (name === "DeleteToLineStart") {
          apply(EDIT_DELETE_TO_LINE_START, "", "change");
          return;
        }
        if (name === "DeleteToLineEnd") {
          apply(EDIT_DELETE_TO_LINE_END, "", "change");
          return;
        }
        if (name === "Enter") {
          if (submitEnter) {
            onSubmit();
          } else {
            apply(EDIT_NEWLINE, "", "change");
          }
        }
      }
    };
    return controller;
  }

  // node_modules/.pnpm/@uifw+solid-ui@file+..+..+frameworks+solid-ui/node_modules/@uifw/solid-ui/src/field/shell.tsx
  var LINE = 22;
  function utf16Len2(value) {
    let count = 0;
    for (const char of value) {
      count += (char.codePointAt(0) ?? 0) > 65535 ? 2 : 1;
    }
    return count;
  }
  function findText(id) {
    let child = host.firstChild(id);
    while (child !== 0) {
      if (host.isText(child)) {
        return child;
      }
      const nested = findText(child);
      if (nested !== 0) {
        return nested;
      }
      child = host.nextSibling(child);
    }
    return 0;
  }
  function maskPassword(raw) {
    return "\u2022".repeat(coreHost.graphemeCount(raw));
  }
  function FieldShell(props) {
    const theme = useTheme();
    const [focused, setFocused] = createSignal2(false);
    const [blink, setBlink] = createSignal2(true);
    const [caretX, setCaretX] = createSignal2(0);
    const [caretY, setCaretY] = createSignal2(0);
    const [caretH, setCaretH] = createSignal2(LINE);
    const [rects, setRects] = createSignal2([]);
    let node = 0;
    let box = 0;
    const controller = createFieldController({
      textEdit: (op, extra) => {
        const id = leaf();
        if (id === 0) {
          return {
            text: "",
            start: 0,
            end: 0
          };
        }
        return host.textEdit(id, op, extra);
      },
      onChange: (value) => props.onChange?.(value),
      validate: props.validate,
      maxLength: () => props.maxLength,
      focus: () => {
        if (node !== 0) {
          host.focus(node);
        }
      },
      onVertical: (direction) => {
        host.focusStep(direction);
      },
      allowChange: () => !frozen()
    });
    props.controller?.(controller);
    const password = () => props.keyboard === "password";
    const multiline = () => !password() && !!props.multiline;
    const blocked2 = () => !!props.disabled;
    const frozen = () => blocked2() || !!props.readOnly;
    const hint = () => controller.text().length === 0 && controller.composing() == null;
    const shownError = () => {
      const override = props.error;
      if (override !== void 0 && override.length > 0) {
        return override;
      }
      return controller.error();
    };
    const shown = () => {
      if (hint()) {
        return props.placeholder ?? "";
      }
      if (password()) {
        return maskPassword(controller.text());
      }
      return controller.text();
    };
    function leaf() {
      return node === 0 ? 0 : findText(node);
    }
    function clusterIndex(raw, utf16) {
      if (!password() || raw.length === 0) {
        return utf16;
      }
      return coreHost.graphemeIndex(raw, utf16);
    }
    function utf16OfCluster(raw, index) {
      if (!password() || raw.length === 0) {
        return index;
      }
      return coreHost.graphemeUnit(raw, index);
    }
    function paint() {
      const id = leaf();
      if (id === 0) {
        return;
      }
      host.setText(id, shown());
    }
    function role() {
      if (props.action === "search") {
        return Role.SearchField;
      }
      if (multiline()) {
        return Role.TextArea;
      }
      return Role.TextField;
    }
    function writeSemantics() {
      if (node === 0) {
        return;
      }
      const editable = !blocked2() && !props.readOnly ? 1 : 0;
      const masked = password();
      host.setFocusable(node, !blocked2());
      host.setSemantics(node, role(), props.label, masked ? void 0 : controller.text(), void 0, masked ? void 0 : controller.selection().start, masked ? void 0 : controller.selection().end, blocked2() ? 1 : 0, editable, props.readOnly ? 1 : 0, shownError() || void 0, multiline() ? {
        order: props.rows
      } : void 0);
    }
    function publish(show) {
      if (!show || blocked2()) {
        host.setImeState(null);
        return;
      }
      const full = controller.text();
      const caret = controller.selection().end;
      const anchor = controller.selection().start;
      const from = controller.composeStart();
      const to = controller.composeEnd();
      const composingStart = from >= 0 && to > from ? from : -1;
      const composingEnd = from >= 0 && to > from ? to : -1;
      const id = leaf();
      let x = caretX();
      let y = caretY();
      let h = caretH();
      if (id !== 0) {
        const textBox = host.rect(id);
        x = textBox.x + caretX();
        y = textBox.y + caretY();
      }
      const config = imeConfig(props.keyboard, props.action, multiline());
      host.setImeConfig(config.keyboard, config.action);
      host.setImeState(full, anchor, caret, composingStart, composingEnd, x, y, 1, h || LINE);
    }
    function follow() {
      const id = leaf();
      if (id === 0) {
        return;
      }
      scrollIntoView(id, 0);
    }
    bindFocusRing(() => box, focused);
    onCleanup(onFrame((targetMs) => {
      if (!focused()) {
        return;
      }
      const id = leaf();
      const raw = controller.text();
      if (id !== 0 && raw.length > 0) {
        const hit = host.caretAtUnit(id, clusterIndex(raw, controller.selection().end));
        setCaretX(hit.x);
        setCaretY(hit.y);
        setCaretH(hit.h || LINE);
      }
      if (host.reduceMotion()) {
        setBlink(true);
        return;
      }
      setBlink(Math.floor(targetMs / 500) % 2 === 0);
      requestFrame();
    }));
    createTrackedEffect(() => {
      const unit = controller.selection().end;
      const value = controller.text();
      const anchor = controller.selection().start;
      shown();
      const id = leaf();
      if (id === 0 || value.length === 0) {
        setCaretX(0);
        setCaretY(0);
        setCaretH(LINE);
        setRects([]);
        paint();
        return;
      }
      const display = clusterIndex(value, unit);
      const hit = host.caretAtUnit(id, display);
      setCaretX(hit.x);
      setCaretY(hit.y);
      setCaretH(hit.h || LINE);
      const from = Math.min(anchor, unit);
      const to = Math.max(anchor, unit);
      if (from === to) {
        setRects([]);
      } else if (password()) {
        setRects(host.selectionRects(id, clusterIndex(value, from), clusterIndex(value, to)));
      } else {
        setRects(host.selectionRects(id, from, to));
      }
      paint();
      follow();
    });
    createTrackedEffect(() => {
      const value = props.value;
      if (value !== void 0) {
        controller.load(value);
        paint();
      }
    });
    createTrackedEffect(() => {
      if (props.mask) {
        controller.setMask(props.mask);
      }
      if (props.transform) {
        controller.setTransform(props.transform);
      }
    });
    createTrackedEffect(() => {
      writeSemantics();
    });
    createTrackedEffect(() => {
      const color = colorBytes2(border());
      if (box !== 0) {
        setStroke(box, 1, color);
      }
    });
    const minHeight = () => {
      if (props.rows !== void 0 && props.rows > 0) {
        return props.rows * LINE;
      }
      return 48;
    };
    const border = () => {
      if (shownError()) {
        return theme().color.danger;
      }
      if (focused()) {
        return theme().color.focusRing;
      }
      return theme().color.outline;
    };
    var _el$ = createElement("column", {
      w: "100%"
    }), _el$2 = createElement("pointer", {
      w: "100%",
      onFocus: () => {
        if (blocked2()) {
          return;
        }
        setFocused(true);
        setBlink(true);
        publish(true);
        follow();
        requestFrame();
      },
      onBlur: () => {
        setFocused(false);
        publish(false);
      },
      onPress: () => {
        if (blocked2()) {
          return;
        }
        if (node !== 0) {
          host.focus(node);
        }
        const id = leaf();
        if (id === 0) {
          return;
        }
        const point = host.pointerPosition();
        const textBox = host.rect(id);
        const inside = point.x >= textBox.x && point.x <= textBox.x + Math.max(textBox.w, 1) && point.y >= textBox.y && point.y <= textBox.y + Math.max(textBox.h, 1);
        if (inside) {
          const caretHit = host.caretAt(id, point.x, point.y);
          controller.caretAt(utf16OfCluster(controller.text(), caretHit.unit));
        } else if (controller.text().length > 0) {
          controller.caretAt(utf16Len2(controller.text()));
        }
        publish(true);
      },
      onDragBy: () => {
        if (blocked2()) {
          return;
        }
        const id = leaf();
        if (id === 0) {
          return;
        }
        const point = host.pointerPosition();
        const caretHit = host.caretAt(id, point.x, point.y);
        controller.extendCaret(utf16OfCluster(controller.text(), caretHit.unit));
        publish(true);
      },
      onKey: (key) => {
        if (blocked2()) {
          return;
        }
        controller.handleKey(key, multiline(), password(), !enterInsertsNewline(multiline(), props.action), () => props.onSubmit?.(controller.text()));
        publish(true);
        paint();
      },
      onText: (value) => {
        if (frozen()) {
          return;
        }
        controller.onText(value);
        publish(true);
        paint();
      },
      onTextReplaced: (value, anchor, caret, composeFrom, composeTo) => {
        if (frozen()) {
          return;
        }
        if (typeof anchor === "number" && typeof caret === "number") {
          controller.replace(value, anchor, caret, typeof composeFrom === "number" ? composeFrom : -1, typeof composeTo === "number" ? composeTo : -1);
        } else {
          controller.onTextReplaced(value);
        }
        publish(focused());
        paint();
      },
      onComposing: (value) => {
        if (frozen()) {
          return;
        }
        controller.onComposing(value);
        publish(true);
        paint();
      }
    }), _el$3 = createElement("decorated", {
      w: "100%"
    }), _el$4 = createElement("opacity"), _el$5 = createElement("padding", {
      w: "100%"
    }), _el$6 = createElement("scroll"), _el$7 = createElement("padding", {
      right: 2
    }), _el$8 = createElement("stack");
    insertNode(_el$, _el$2);
    insertNode(_el$2, _el$3);
    ref(() => (handle) => {
      node = handle.id;
      host.setDragClaim(handle.id, multiline() ? "none" : "horizontal");
      if (multiline()) {
        setRecognizers(handle.id, {
          longPress: true,
          drag: "none"
        });
      }
      host.setFocusable(handle.id, !blocked2());
      writeSemantics();
    }, _el$2);
    insertNode(_el$3, _el$4);
    ref(() => (handle) => {
      box = handle.id;
      setStroke(handle.id, 1, colorBytes2(border()));
    }, _el$3);
    insertNode(_el$4, _el$5);
    insertNode(_el$5, _el$6);
    insertNode(_el$6, _el$7);
    insertNode(_el$7, _el$8);
    insert(_el$8, createComponent2(Text, {
      get fontSize() {
        return theme().typography.body.size;
      },
      get color() {
        return memo2(() => !!hint())() ? theme().color.textMuted : theme().color.text;
      },
      get weight() {
        return theme().typography.body.weight;
      },
      get direction() {
        return directionFor(theme().locale);
      },
      get softWrap() {
        return multiline();
      },
      get overflow() {
        return multiline() ? "clip" : "visible";
      },
      get maxLines() {
        return memo2(() => !!multiline())() ? props.maxLines : void 0;
      },
      get children() {
        return shown();
      }
    }), null);
    insert(_el$8, createComponent2(CaretLayer, {
      rects,
      caretX,
      caretY,
      caretH,
      showCaret: () => focused() && blink() && controller.selection().start === controller.selection().end,
      accent: () => theme().color.accent,
      transparent: () => theme().color.transparent
    }), null);
    insert(_el$, (() => {
      var _c$ = memo2(() => !!shownError());
      return () => _c$() ? createComponent2(Text, {
        get fontSize() {
          return theme().typography.body.size;
        },
        get color() {
          return theme().color.danger;
        },
        get weight() {
          return theme().typography.body.weight;
        },
        get direction() {
          return directionFor(theme().locale);
        },
        get children() {
          return shownError();
        }
      }) : null;
    })(), null);
    effect3(() => ({
      e: theme().space.s,
      t: {
        h: minHeight()
      },
      a: theme().color.surface,
      o: theme().shape.l,
      i: {
        h: minHeight()
      },
      n: blocked2() ? 102 : 255,
      s: theme().space.l,
      h: multiline() ? "vertical" : "horizontal"
    }), ({
      e,
      t: t2,
      a,
      o,
      i,
      n,
      s,
      h
    }, _p$) => {
      e !== _p$?.e && setProp(_el$, "gap", e, _p$?.e);
      t2 !== _p$?.t && setProp(_el$2, "min", t2, _p$?.t);
      a !== _p$?.a && setProp(_el$3, "color", a, _p$?.a);
      o !== _p$?.o && setProp(_el$3, "radius", o, _p$?.o);
      i !== _p$?.i && setProp(_el$3, "min", i, _p$?.i);
      n !== _p$?.n && setProp(_el$4, "alpha", n, _p$?.n);
      s !== _p$?.s && setProp(_el$5, "edges", s, _p$?.s);
      h !== _p$?.h && setProp(_el$6, "axis", h, _p$?.h);
    });
    return _el$;
  }

  // node_modules/.pnpm/@uifw+solid-ui@file+..+..+frameworks+solid-ui/node_modules/@uifw/solid-ui/src/Field.tsx
  function Field(props) {
    return createComponent2(FieldShell, props);
  }

  // node_modules/.pnpm/@uifw+solid-ui@file+..+..+frameworks+solid-ui/node_modules/@uifw/solid-ui/src/overlay/OverlayRoot.tsx
  var api = {
    register: registerOverlay,
    closeTopModal,
    entries: overlayEntries
  };
  var OverlayContext = createContext2(api);
  function OverlayRoot(props) {
    return createComponent2(OverlayContext, {
      value: api,
      get children() {
        var _el$ = createElement("stack", {
          fill: true
        });
        insert(_el$, () => props.children);
        return _el$;
      }
    });
  }

  // node_modules/.pnpm/@uifw+solid-ui@file+..+..+frameworks+solid-ui/node_modules/@uifw/solid-ui/src/ScrollController.ts
  var natives = /* @__PURE__ */ new WeakMap();
  function createScrollController() {
    const native = host.scrollController?.();
    if (native === void 0) {
      throw new Error("scrollController is not linked");
    }
    return wrapScroll(native);
  }
  function wrapScroll(native) {
    let viewport = 0;
    let content = 0;
    let contentMin = 0;
    let pullMode = 0;
    let threshold = 64;
    let rubber = false;
    let factor = 0.55;
    let parent = null;
    const [phase, setPhase] = createSignal2("idle");
    const [overscroll, setOverscroll] = createSignal2({
      x: 0,
      y: 0
    });
    const offsetListeners = /* @__PURE__ */ new Set();
    let stopState = () => {
    };
    function push() {
      native.configure(contentMin, pullMode, threshold, rubber, factor, parent);
    }
    function readOver() {
      const sample = native.query(2);
      setOverscroll({
        x: sample.x,
        y: sample.y
      });
    }
    const controller = {
      node: () => viewport,
      bind(next) {
        stopState();
        viewport = next;
        if (next !== 0) {
          native.bind(next, content);
          stopState = onScrollState(next, (sample) => {
            setPhase(sample.phase);
            setOverscroll({
              x: sample.ox,
              y: sample.oy
            });
          });
        }
      },
      bindContent(next) {
        content = next;
        if (viewport !== 0) {
          native.bind(viewport, next);
        }
      },
      phase,
      overscroll,
      offset() {
        const sample = native.query(0);
        return {
          x: sample.x,
          y: sample.y
        };
      },
      onOffset(fn2) {
        offsetListeners.add(fn2);
        return () => {
          offsetListeners.delete(fn2);
        };
      },
      noteOffset(offset) {
        for (const fn2 of offsetListeners) {
          fn2(offset);
        }
      },
      setOverscroll(next) {
        setOverscroll(next);
      },
      limits() {
        const sample = native.query(1);
        return {
          x: sample.x,
          y: sample.y
        };
      },
      setLimits(_next) {
      },
      scrollTo(x, y, animate2) {
        if (viewport === 0) {
          return;
        }
        host.scrollTo(viewport, x, y, animate2?.stiffness ?? 0, animate2?.damping ?? 0);
      },
      fling(vx, vy) {
        if (viewport === 0) {
          return;
        }
        host.fling(viewport, vx, vy);
      },
      stop() {
        if (viewport === 0) {
          return;
        }
        host.stopScroll(viewport);
      },
      dragBy(dx, dy) {
        const step = native.dragBy(dx, dy);
        if (!step.pending) {
          readOver();
        }
        return step;
      },
      finishDrag(taken) {
        native.finishDrag(taken);
        readOver();
      },
      dragEnd() {
        native.dragEnd();
      },
      wheel(dx, dy) {
        native.wheel(dx, dy);
      },
      setContentMin(height) {
        contentMin = height;
        push();
      },
      setPull(mode, nextThreshold) {
        pullMode = mode;
        threshold = nextThreshold;
        push();
      },
      setRubber(nextRubber, nextFactor) {
        rubber = nextRubber;
        factor = nextFactor;
        push();
      },
      setParent(next) {
        parent = next === null ? null : natives.get(next) ?? null;
        push();
      },
      consume(overflowX, overflowY) {
        const taken = native.consume(overflowX, overflowY);
        if (taken) {
          const sample = native.query(0);
          controller.noteOffset({
            x: sample.x,
            y: sample.y
          });
        }
        return taken;
      },
      nativeId() {
        return native.id();
      }
    };
    natives.set(controller, native);
    onCleanup(() => {
      stopState();
      native.release();
    });
    return controller;
  }
  function refreshListener(controller, fn2) {
    return listenRefresh(controller.nativeId(), fn2);
  }

  // node_modules/.pnpm/@uifw+solid-ui@file+..+..+frameworks+solid-ui/node_modules/@uifw/solid-ui/src/ScrollArea.tsx
  function acceptCode(accept) {
    if (accept === "always") {
      return 1;
    }
    if (accept === "never") {
      return 2;
    }
    return 0;
  }
  function ScrollArea(props) {
    const ctrl = props.controller ?? createScrollController();
    let viewport = 0;
    let content = 0;
    function applyContentMin() {
      const min = props.contentMin ?? 0;
      ctrl.setContentMin(min);
      if (content !== 0 && Number.isFinite(min) && min > 0) {
        host.setMinSize(content, 0, min);
      }
    }
    function applyPhysics(node) {
      const physics = props.physics;
      const align = (name) => name === "center" ? 1 : name === "end" ? 2 : 0;
      host.setScrollPhysics(node, physics?.frictionTau ?? 180, physics?.overscroll === "clamp" ? 1 : 0, physics?.stiffness ?? 170, physics?.damping ?? 26, physics?.factor ?? 0.55, physics?.snapX?.interval ?? 0, align(physics?.snapX?.align), physics?.snapY?.interval ?? 0, align(physics?.snapY?.align), acceptCode(physics?.accept ?? "always"));
      if (physics?.chain !== void 0) {
        host.setScrollChain(node, physics.chain);
      }
      ctrl.setRubber(physics?.overscroll === "rubber", physics?.factor ?? 0.55);
    }
    createTrackedEffect(() => {
      applyContentMin();
    });
    var _el$ = createElement("scroll", {
      fill: true,
      onDragBy: (dx, dy) => {
        const step = ctrl.dragBy(dx, dy);
        if (!step.pending) {
          return;
        }
        const taken = props.onDragOverflow?.(step.ox, step.oy) === true;
        ctrl.finishDrag(taken);
      },
      onDragEnd: () => {
        ctrl.dragEnd();
        props.onDragEnd?.();
      },
      onWheel: (dx, dy) => {
        ctrl.wheel(dx, dy);
      }
    }), _el$2 = createElement("column");
    insertNode(_el$, _el$2);
    ref(() => (node) => {
      viewport = node.id;
      ctrl.bind(node.id);
      applyPhysics(node.id);
      const initial = props.initialOffset;
      if (initial !== void 0 && (initial.x !== 0 || initial.y !== 0)) {
        host.setScrollOffset(viewport, initial.x, initial.y);
        ctrl.bind(node.id);
      }
      props.viewportRef?.(node.id);
    }, _el$);
    ref(() => (node) => {
      content = node.id;
      ctrl.bindContent(node.id);
      applyContentMin();
      props.contentRef?.(node.id);
    }, _el$2);
    insert(_el$2, () => props.children);
    effect3(() => props.axis ?? "vertical", (_v$, _$p) => {
      setProp(_el$, "axis", _v$, _$p);
    });
    return _el$;
  }

  // node_modules/.pnpm/@uifw+solid-ui@file+..+..+frameworks+solid-ui/node_modules/@uifw/solid-ui/src/RefreshControl.tsx
  var DEFAULT_THRESHOLD = 64;
  function RefreshControl(props) {
    createRenderEffect2(() => props.threshold ?? DEFAULT_THRESHOLD, (threshold) => {
      props.controller.setPull(1, threshold);
    });
    onCleanup(refreshListener(props.controller, () => props.onRefresh?.()));
    const pull = () => Math.max(0, -props.controller.overscroll().y);
    var _el$ = createElement("column"), _el$2 = createElement("sized", {
      w: 1
    });
    insertNode(_el$, _el$2);
    insert(_el$, () => props.children, null);
    effect3(() => pull(), (_v$, _$p) => {
      setProp(_el$2, "h", _v$, _$p);
    });
    return _el$;
  }

  // node_modules/.pnpm/@uifw+solid-ui@file+..+..+frameworks+solid-ui/node_modules/@uifw/solid-ui/src/motion-clock.ts
  var bound = null;
  function bindMotion(next) {
    bound = next;
  }
  function animateOnClock(hooks2, target, durationMs, write) {
    let start = null;
    let stop = () => {
    };
    stop = hooks2.onFrame((targetMs) => {
      if (start === null) {
        start = targetMs;
      }
      const t2 = Math.min(1, Math.max(0, (targetMs - start) / durationMs));
      write(target * t2);
      if (t2 >= 1) {
        stop();
      }
    });
    hooks2.requestFrame();
    return () => {
      stop();
    };
  }

  // node_modules/.pnpm/@uifw+solid-ui@file+..+..+frameworks+solid-ui/node_modules/@uifw/solid-ui/src/motion.ts
  var hooks = {
    reduceMotion: () => {
      const core = globalThis.core;
      if (typeof core?.reduceMotion !== "function") {
        return false;
      }
      return core.reduceMotion();
    },
    onFrame,
    requestFrame,
    animate: (target, durationMs, write) => animateOnClock(hooks, target, durationMs, write)
  };
  bindMotion(hooks);

  // node_modules/.pnpm/@uifw+solid-ui@file+..+..+frameworks+solid-ui/node_modules/@uifw/solid-ui/src/VirtualList.tsx
  var restoredOffsets = /* @__PURE__ */ new Map();
  function VirtualList(props) {
    const theme = useTheme();
    const scroll = props.controller ?? createScrollController();
    const native = host.listDriver?.(props.overscan ?? 4, props.estimatedHeight);
    if (native === void 0) {
      throw new Error("listDriver is not linked");
    }
    const initial = props.restoreKey ? restoredOffsets.get(props.restoreKey) : void 0;
    const [extent, setExtent] = createSignal2(0);
    const [thumbOn, setThumbOn] = createSignal2(false);
    const [thumbH, setThumbH] = createSignal2(24);
    const [hThumbOn, setHThumbOn] = createSignal2(false);
    const [hThumbW, setHThumbW] = createSignal2(24);
    const [pin, setPin] = createSignal2(null);
    let viewport = 0;
    let contentId = 0;
    let top = 0;
    let bottom = 0;
    let rows = 0;
    let vThumb = 0;
    let hThumb = 0;
    let pinNode = 0;
    let vSize = 0;
    let hSize = 0;
    function syncNodes() {
      native.bind(viewport, contentId, top, bottom, rows, vThumb, hThumb, pinNode, vSize, hSize);
    }
    createTrackedEffect(() => {
      const items = props.items;
      native.configure(props.estimatedHeight, props.overscan ?? 4, (props.error ?? "").length > 0);
      if (props.onRefresh) {
        scroll.setPull(2, 64);
      }
      native.setItems(items.map((item, index) => ({
        key: String(props.itemKey(item)),
        type: props.recycle ? props.recycle.typeOf(item) : "item",
        sticky: props.sticky?.(item, index) === true
      })));
    });
    onCleanup(bindListRenderer(native, {
      parent: () => rows,
      poolSize: () => props.recycle ? props.recycle.poolSize ?? 8 : 0,
      itemAt: (index) => props.items[index],
      render: (item, index) => {
        if (props.recycle) {
          return props.recycle.render(item, index);
        }
        return props.children?.(item(), index()) ?? createElement("sized", {
          w: 0,
          h: 0
        });
      },
      createSlot,
      onChrome(chrome) {
        setExtent(chrome.extent);
        setThumbOn(chrome.thumbOn);
        setThumbH(chrome.thumbLen);
        setHThumbOn(chrome.hThumbOn);
        setHThumbW(chrome.hThumbLen);
        if (chrome.pinIndex >= 0) {
          setPin({
            index: chrome.pinIndex,
            key: chrome.pinKey
          });
        } else {
          setPin(null);
        }
      },
      onEnd() {
        if ((props.error ?? "").length > 0) {
          return;
        }
        props.onEndReached?.();
      }
    }));
    onCleanup(refreshListener(scroll, () => props.onRefresh?.()));
    onCleanup(() => {
      const key = props.restoreKey;
      if (key) {
        restoredOffsets.set(key, scroll.offset());
      }
      native.release();
    });
    const errored = () => (props.error ?? "").length > 0;
    const pinned = () => {
      const box = pin();
      if (box === null) {
        return null;
      }
      return props.items[box.index] === void 0 ? null : box;
    };
    return createComponent2(Show, {
      get when() {
        return !errored();
      },
      get fallback() {
        var _el$14 = createElement("column");
        insert(_el$14, createComponent2(Text, {
          get fontSize() {
            return theme().fontSize.body;
          },
          get color() {
            return theme().color.danger;
          },
          get direction() {
            return directionFor(theme().locale);
          },
          get children() {
            return props.error ?? "";
          }
        }), null);
        insert(_el$14, createComponent2(Show, {
          get when() {
            return props.onRetry;
          },
          get children() {
            return createComponent2(Button, {
              label: "Retry",
              onClick: () => props.onRetry?.()
            });
          }
        }), null);
        effect3(() => theme().space.s, (_v$, _$p) => {
          setProp(_el$14, "gap", _v$, _$p);
        });
        return _el$14;
      },
      get children() {
        return createComponent2(Show, {
          get when() {
            return props.items.length > 0;
          },
          get fallback() {
            return memo2(() => !!props.skeleton)() ? props.skeleton() : memo2(() => !!props.empty)() ? props.empty() : createElement("sized", {
              w: 0,
              h: 0
            });
          },
          get children() {
            var _el$2 = createElement("stack", {
              fill: true
            });
            insert(_el$2, createComponent2(ScrollArea, {
              axis: "vertical",
              get contentMin() {
                return extent();
              },
              initialOffset: initial,
              controller: scroll,
              get physics() {
                return props.physics;
              },
              get onDragOverflow() {
                return props.onDragOverflow;
              },
              viewportRef: (id) => {
                viewport = id;
                syncNodes();
              },
              contentRef: (id) => {
                contentId = id;
                syncNodes();
              },
              get children() {
                return [(() => {
                  var _el$3 = createElement("sized", {
                    w: 1,
                    h: 0
                  });
                  ref(() => (node) => {
                    top = node.id;
                    syncNodes();
                  }, _el$3);
                  return _el$3;
                })(), (() => {
                  var _el$4 = createElement("column");
                  ref(() => (node) => {
                    rows = node.id;
                    syncNodes();
                  }, _el$4);
                  return _el$4;
                })(), (() => {
                  var _el$5 = createElement("sized", {
                    w: 1,
                    h: 0
                  });
                  ref(() => (node) => {
                    bottom = node.id;
                    syncNodes();
                  }, _el$5);
                  return _el$5;
                })()];
              }
            }), null);
            insert(_el$2, createComponent2(Show, {
              get when() {
                return pinned();
              },
              keyed: true,
              children: (box) => (() => {
                var _el$16 = createElement("transform");
                ref(() => (node) => {
                  pinNode = node.id;
                  syncNodes();
                }, _el$16);
                insert(_el$16, (() => {
                  var _c$ = memo2(() => !!props.recycle);
                  return () => _c$() ? props.recycle.render(() => props.items[box.index], () => box.index) : props.children?.(props.items[box.index], box.index);
                })());
                return _el$16;
              })()
            }), null);
            insert(_el$2, createComponent2(Show, {
              get when() {
                return thumbOn();
              },
              get children() {
                var _el$6 = createElement("row", {
                  fill: true,
                  justify: "end"
                }), _el$7 = createElement("padding"), _el$8 = createElement("transform"), _el$9 = createElement("decorated"), _el$0 = createElement("sized", {
                  w: 3
                });
                insertNode(_el$6, _el$7);
                insertNode(_el$7, _el$8);
                insertNode(_el$8, _el$9);
                ref(() => (node) => {
                  vThumb = node.id;
                  syncNodes();
                }, _el$8);
                insertNode(_el$9, _el$0);
                ref(() => (node) => {
                  vSize = node.id;
                  syncNodes();
                }, _el$0);
                effect3(() => ({
                  e: theme().space.s,
                  t: theme().color.textMuted,
                  a: theme().radius.s,
                  o: thumbH()
                }), ({
                  e,
                  t: t2,
                  a,
                  o
                }, _p$) => {
                  e !== _p$?.e && setProp(_el$7, "right", e, _p$?.e);
                  t2 !== _p$?.t && setProp(_el$9, "color", t2, _p$?.t);
                  a !== _p$?.a && setProp(_el$9, "radius", a, _p$?.a);
                  o !== _p$?.o && setProp(_el$0, "h", o, _p$?.o);
                });
                return _el$6;
              }
            }), null);
            insert(_el$2, createComponent2(Show, {
              get when() {
                return hThumbOn();
              },
              get children() {
                var _el$1 = createElement("column", {
                  fill: true,
                  justify: "end"
                }), _el$10 = createElement("padding"), _el$11 = createElement("transform"), _el$12 = createElement("decorated"), _el$13 = createElement("sized", {
                  h: 3
                });
                insertNode(_el$1, _el$10);
                insertNode(_el$10, _el$11);
                insertNode(_el$11, _el$12);
                ref(() => (node) => {
                  hThumb = node.id;
                  syncNodes();
                }, _el$11);
                insertNode(_el$12, _el$13);
                ref(() => (node) => {
                  hSize = node.id;
                  syncNodes();
                }, _el$13);
                effect3(() => ({
                  e: theme().space.s,
                  t: theme().color.textMuted,
                  a: theme().radius.s,
                  o: hThumbW()
                }), ({
                  e,
                  t: t2,
                  a,
                  o
                }, _p$) => {
                  e !== _p$?.e && setProp(_el$10, "bottom", e, _p$?.e);
                  t2 !== _p$?.t && setProp(_el$12, "color", t2, _p$?.t);
                  a !== _p$?.a && setProp(_el$12, "radius", a, _p$?.a);
                  o !== _p$?.o && setProp(_el$13, "w", o, _p$?.o);
                });
                return _el$1;
              }
            }), null);
            return _el$2;
          }
        });
      }
    });
  }

  // util.js
  function kicker() {
    return "Future";
  }

  // counter.tsx
  function Screen(props) {
    const theme = useTheme();
    var _el$ = createElement("decorated", {
      fill: true
    }), _el$2 = createElement("padding", {
      top: 12,
      right: 20,
      bottom: 16,
      left: 20,
      fill: true
    }), _el$3 = createElement("column", {
      fill: true,
      w: "100%"
    });
    insertNode(_el$, _el$2);
    insertNode(_el$2, _el$3);
    insert(_el$3, () => props.children);
    effect3(() => ({
      e: theme().color.background,
      t: theme().space.l
    }), ({
      e,
      t: t2
    }, _p$) => {
      e !== _p$?.e && setProp(_el$, "color", e, _p$?.e);
      t2 !== _p$?.t && setProp(_el$3, "gap", t2, _p$?.t);
    });
    return _el$;
  }
  function Title(props) {
    const theme = useTheme();
    const direction = () => directionFor(theme().locale);
    var _el$4 = createElement("column");
    insert(_el$4, createComponent2(Text, {
      get fontSize() {
        return theme().typography.caption.size;
      },
      get color() {
        return theme().color.textMuted;
      },
      get weight() {
        return theme().typography.caption.weight;
      },
      get direction() {
        return direction();
      },
      get children() {
        return kicker();
      }
    }), null);
    insert(_el$4, createComponent2(Text, {
      get fontSize() {
        return theme().typography.headline.size;
      },
      get color() {
        return theme().color.text;
      },
      get weight() {
        return theme().typography.headline.weight;
      },
      get direction() {
        return direction();
      },
      get children() {
        return props.title;
      }
    }), null);
    effect3(() => theme().space.s, (_v$, _$p) => {
      setProp(_el$4, "gap", _v$, _$p);
    });
    return _el$4;
  }
  function TabBar(props) {
    const theme = useTheme();
    const selected = () => {
      const list2 = props.router.stack();
      const top = list2[list2.length - 1];
      const path = top?.path ?? "";
      if (path.startsWith("/list")) {
        return "list";
      }
      if (path.startsWith("/nav")) {
        return "nav";
      }
      return "schedule";
    };
    var _el$5 = createElement("decorated", {
      w: "100%"
    }), _el$6 = createElement("padding", {
      w: "100%"
    }), _el$7 = createElement("row", {
      justify: "space-evenly",
      w: "100%"
    });
    insertNode(_el$5, _el$6);
    insertNode(_el$6, _el$7);
    insert(_el$7, createComponent2(Button, {
      label: "\u8C03\u5EA6",
      get variant() {
        return selected() === "schedule" ? "primary" : "quiet";
      },
      onClick: () => props.router.reset("/schedule")
    }), null);
    insert(_el$7, createComponent2(Button, {
      label: "\u5BFC\u822A",
      get variant() {
        return selected() === "nav" ? "primary" : "quiet";
      },
      onClick: () => props.router.reset("/nav/1")
    }), null);
    insert(_el$7, createComponent2(Button, {
      label: "\u5217\u8868",
      get variant() {
        return selected() === "list" ? "primary" : "quiet";
      },
      onClick: () => props.router.reset("/list")
    }), null);
    effect3(() => ({
      e: theme().color.surface,
      t: theme().shape.full,
      a: theme().space.s,
      o: theme().space.m
    }), ({
      e,
      t: t2,
      a,
      o
    }, _p$) => {
      e !== _p$?.e && setProp(_el$5, "color", e, _p$?.e);
      t2 !== _p$?.t && setProp(_el$5, "radius", t2, _p$?.t);
      a !== _p$?.a && setProp(_el$6, "edges", a, _p$?.a);
      o !== _p$?.o && setProp(_el$7, "gap", o, _p$?.o);
    });
    return _el$5;
  }
  function SchedulePage(_props) {
    const theme = useTheme();
    const [label, setLabel] = createSignal2("\u8FD8\u662F\u65E7\u7684");
    var _el$8 = createElement("column", {
      w: "100%"
    });
    insert(_el$8, createComponent2(Title, {
      title: "\u8C03\u5EA6"
    }), null);
    insert(_el$8, createComponent2(Text, {
      get fontSize() {
        return theme().typography.display.size;
      },
      get color() {
        return theme().color.text;
      },
      get weight() {
        return theme().typography.display.weight;
      },
      get direction() {
        return directionFor(theme().locale);
      },
      get children() {
        return label();
      }
    }), null);
    insert(_el$8, createComponent2(Text, {
      get fontSize() {
        return theme().typography.body.size;
      },
      get color() {
        return theme().color.textMuted;
      },
      get direction() {
        return directionFor(theme().locale);
      },
      children: "\u5FAE\u4EFB\u52A1\u5728\u4E0B\u4E00\u5E27\u628A\u8FD9\u6BB5\u6587\u5B57\u5199\u56DE\u6765\u3002"
    }), null);
    insert(_el$8, createComponent2(Field, {
      placeholder: "\u6807\u7B7E",
      get value() {
        return label();
      },
      onChange: setLabel
    }), null);
    insert(_el$8, createComponent2(Button, {
      wide: true,
      variant: "primary",
      label: "\u5FAE\u4EFB\u52A1\u6539\u5B57",
      onClick: () => {
        queueMicrotask(() => setLabel("\u5DF2\u7ECF\u6539\u4E86"));
      }
    }), null);
    effect3(() => theme().space.l, (_v$, _$p) => {
      setProp(_el$8, "gap", _v$, _$p);
    });
    return _el$8;
  }
  var LIST_ITEMS = Array.from({
    length: 200
  }, (_, n) => ({
    n
  }));
  function ListPage(_props) {
    const theme = useTheme();
    const list2 = createScrollController();
    const [scale, setScale] = createSignal2(1);
    const [missing, setMissing] = createSignal2(false);
    const [name, setName] = createSignal2("");
    const [refreshed, setRefreshed] = createSignal2(0);
    var _el$9 = createElement("column", {
      fill: true,
      w: "100%"
    }), _el$0 = createElement("row", {
      w: "100%"
    }), _el$10 = createElement("clip", {
      fill: true
    }), _el$11 = createElement("decorated", {
      fill: true
    });
    insertNode(_el$9, _el$0);
    insertNode(_el$9, _el$10);
    insert(_el$9, createComponent2(Title, {
      title: "\u5217\u8868"
    }), _el$0);
    insert(_el$0, createComponent2(Button, {
      wide: true,
      get label() {
        return scale() === 1 ? "\u5B57\u53F7 2.0" : "\u5B57\u53F7 1.0";
      },
      onClick: () => {
        const next = scale() === 1 ? 2 : 1;
        setScale(next);
        host.setTextScale(next);
      }
    }), null);
    insert(_el$0, createComponent2(Button, {
      wide: true,
      get label() {
        return missing() ? "\u53BB\u6389\u7F3A\u56FE" : "\u7F3A\u56FE";
      },
      onClick: () => setMissing(!missing())
    }), null);
    insert(_el$9, createComponent2(Show, {
      get when() {
        return missing();
      },
      get children() {
        return createElement("image", {
          source: "asset:missing",
          w: 32,
          h: 32
        });
      }
    }), _el$10);
    insert(_el$9, createComponent2(Field, {
      placeholder: "\u59D3\u540D",
      onChange: setName
    }), _el$10);
    insert(_el$9, createComponent2(Text, {
      get fontSize() {
        return theme().typography.body.size;
      },
      get color() {
        return theme().color.text;
      },
      get direction() {
        return directionFor(theme().locale);
      },
      get children() {
        return name() || "\u672A\u586B\u5199";
      }
    }), _el$10);
    insertNode(_el$10, _el$11);
    insert(_el$11, createComponent2(RefreshControl, {
      controller: list2,
      onRefresh: () => setRefreshed((count) => count + 1),
      get children() {
        return [createComponent2(Text, {
          get fontSize() {
            return theme().typography.caption.size;
          },
          get color() {
            return theme().color.textMuted;
          },
          get direction() {
            return directionFor(theme().locale);
          },
          get children() {
            return memo2(() => refreshed() === 0)() ? "\u4E0B\u62C9\u5237\u65B0" : `\u5DF2\u5237\u65B0 ${refreshed()}`;
          }
        }), createComponent2(Text, {
          get fontSize() {
            return theme().typography.caption.size;
          },
          get color() {
            return theme().color.textMuted;
          },
          get direction() {
            return directionFor(theme().locale);
          },
          children: "\u6A2A\u5411\u5206\u9875"
        }), createComponent2(ScrollArea, {
          axis: "horizontal",
          physics: {
            snapX: {
              interval: 72,
              align: "start"
            },
            overscroll: "clamp"
          },
          get children() {
            var _el$12 = createElement("row", {
              gap: 8
            }), _el$13 = createElement("sized", {
              w: 72,
              h: 36
            }), _el$14 = createElement("sized", {
              w: 72,
              h: 36
            }), _el$15 = createElement("sized", {
              w: 72,
              h: 36
            }), _el$16 = createElement("sized", {
              w: 72,
              h: 36
            }), _el$17 = createElement("sized", {
              w: 72,
              h: 36
            });
            insertNode(_el$12, _el$13);
            insertNode(_el$12, _el$14);
            insertNode(_el$12, _el$15);
            insertNode(_el$12, _el$16);
            insertNode(_el$12, _el$17);
            return _el$12;
          }
        })];
      }
    }), null);
    insert(_el$11, createComponent2(VirtualList, {
      items: LIST_ITEMS,
      itemKey: (item) => item.n,
      estimatedHeight: 48,
      controller: list2,
      physics: {
        overscroll: "rubber"
      },
      recycle: {
        typeOf: () => "row",
        render: (item) => (() => {
          var _el$18 = createElement("padding");
          insert(_el$18, createComponent2(Text, {
            get fontSize() {
              return theme().typography.body.size;
            },
            get color() {
              return theme().color.text;
            },
            get direction() {
              return directionFor(theme().locale);
            },
            get children() {
              return `Item ${item().n}`;
            }
          }));
          effect3(() => ({
            e: theme().space.l,
            t: theme().space.l,
            a: theme().space.m,
            o: theme().space.m
          }), ({
            e,
            t: t2,
            a,
            o
          }, _p$) => {
            e !== _p$?.e && setProp(_el$18, "left", e, _p$?.e);
            t2 !== _p$?.t && setProp(_el$18, "right", t2, _p$?.t);
            a !== _p$?.a && setProp(_el$18, "top", a, _p$?.a);
            o !== _p$?.o && setProp(_el$18, "bottom", o, _p$?.o);
          });
          return _el$18;
        })()
      }
    }), null);
    effect3(() => ({
      e: theme().space.l,
      t: theme().space.s,
      a: theme().shape.l,
      o: theme().color.surface
    }), ({
      e,
      t: t2,
      a,
      o
    }, _p$) => {
      e !== _p$?.e && setProp(_el$9, "gap", e, _p$?.e);
      t2 !== _p$?.t && setProp(_el$0, "gap", t2, _p$?.t);
      a !== _p$?.a && setProp(_el$10, "radius", a, _p$?.a);
      o !== _p$?.o && setProp(_el$11, "color", o, _p$?.o);
    });
    return _el$9;
  }
  function App() {
    const theme = useTheme();
    const router = createRouter([{
      path: "/schedule",
      component: SchedulePage
    }, {
      path: "/nav/:depth",
      component: (props) => {
        const depth = Number(props.params.depth ?? "1") || 1;
        const [dialog, setDialog] = createSignal2(false);
        var _el$19 = createElement("column", {
          w: "100%"
        }), _el$20 = createElement("row", {
          w: "100%"
        });
        insertNode(_el$19, _el$20);
        insert(_el$19, createComponent2(Title, {
          title: `\u7B2C ${depth} \u9875`
        }), _el$20);
        insert(_el$19, createComponent2(Text, {
          get fontSize() {
            return theme().typography.body.size;
          },
          get color() {
            return theme().color.textMuted;
          },
          get direction() {
            return directionFor(theme().locale);
          },
          children: "\u63A8\u8FDB\u4F1A\u538B\u5165\u4E0B\u4E00\u5C42\uFF0C\u5F39\u51FA\u7B49\u8F6C\u573A\u7ED3\u675F\u518D\u5378\u4E0B\u3002"
        }), _el$20);
        insert(_el$19, createComponent2(Button, {
          wide: true,
          variant: "primary",
          label: "\u63A8\u8FDB",
          onClick: () => router.push(`/nav/${depth + 1}`)
        }), _el$20);
        insert(_el$20, createComponent2(Button, {
          wide: true,
          label: "\u5F39\u51FA",
          onClick: () => router.pop()
        }), null);
        insert(_el$20, createComponent2(Button, {
          wide: true,
          label: "\u5BF9\u8BDD\u6846",
          onClick: () => setDialog(true)
        }), null);
        insert(_el$19, createComponent2(Dialog, {
          get open() {
            return dialog();
          },
          onClose: () => setDialog(false),
          get children() {
            var _el$21 = createElement("decorated"), _el$22 = createElement("padding"), _el$23 = createElement("column", {
              w: "100%"
            });
            insertNode(_el$21, _el$22);
            insertNode(_el$22, _el$23);
            insert(_el$23, createComponent2(Text, {
              get fontSize() {
                return theme().typography.title.size;
              },
              get color() {
                return theme().color.text;
              },
              get weight() {
                return theme().typography.title.weight;
              },
              get direction() {
                return directionFor(theme().locale);
              },
              children: "\u8FD4\u56DE\u952E\u5173\u95ED"
            }), null);
            insert(_el$23, createComponent2(Button, {
              wide: true,
              variant: "primary",
              label: "\u5173\u95ED",
              onClick: () => setDialog(false)
            }), null);
            effect3(() => ({
              e: theme().color.surface,
              t: theme().shape.l,
              a: theme().space.l,
              o: theme().space.l
            }), ({
              e,
              t: t2,
              a,
              o
            }, _p$) => {
              e !== _p$?.e && setProp(_el$21, "color", e, _p$?.e);
              t2 !== _p$?.t && setProp(_el$21, "radius", t2, _p$?.t);
              a !== _p$?.a && setProp(_el$22, "edges", a, _p$?.a);
              o !== _p$?.o && setProp(_el$23, "gap", o, _p$?.o);
            });
            return _el$21;
          }
        }), null);
        effect3(() => ({
          e: theme().space.l,
          t: theme().space.s
        }), ({
          e,
          t: t2
        }, _p$) => {
          e !== _p$?.e && setProp(_el$19, "gap", e, _p$?.e);
          t2 !== _p$?.t && setProp(_el$20, "gap", t2, _p$?.t);
        });
        return _el$19;
      }
    }, {
      path: "/list",
      component: ListPage
    }], {
      initial: "/schedule"
    });
    return createComponent2(OverlayRoot, {
      get children() {
        return createComponent2(Screen, {
          get children() {
            return [createComponent2(TabBar, {
              router
            }), (() => {
              var _el$24 = createElement("stack", {
                fill: true
              });
              insert(_el$24, createComponent2(RouterView, {
                router,
                get background() {
                  return theme().color.background;
                },
                errorFallback: (err, reset2) => createComponent2(ErrorPage, {
                  get message() {
                    return messageOf(typeof err === "function" ? err() : err);
                  },
                  get stack() {
                    return stackOf(typeof err === "function" ? err() : err);
                  },
                  retry: () => reset2(),
                  onBack: () => router.pop()
                })
              }));
              return _el$24;
            })()];
          }
        });
      }
    });
  }
  boot(App);
})();
//# sourceMappingURL=counter.js.map

(() => {
  // node_modules/.pnpm/@solidjs+signals@2.0.0-rc.17/node_modules/@solidjs/signals/dist/prod/core/error.js
  var NotReadyError = class extends Error {
    constructor(r) {
      const o = Error;
      const t = o.stackTraceLimit;
      if (t !== void 0) o.stackTraceLimit = 0;
      super();
      if (t !== void 0) o.stackTraceLimit = t;
      this.source = r;
    }
  };
  var StatusError = class extends Error {
    constructor(r, o) {
      try {
        var t = String(o instanceof Error ? o.message : o);
      } catch {
      }
      super(t, {
        cause: o
      });
      this.source = r;
    }
  };
  function unwrapStatusError(r) {
    return r instanceof StatusError ? r.cause : r;
  }

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
    const t = e.R;
    if (t & REACTIVE_DISPOSED) return;
    if (n && !r && e.T !== null && (e.T.Ue !== null || e.T.me !== null)) disposeChildren(e, false, true);
    if (n) {
      e.R = t | REACTIVE_DISPOSED;
    }
    if (n && e.X) {
      if (e.T !== null) e.T.Se = null;
      if (e.m & STATUS_PENDING && e.T?.Re != null || t & REACTIVE_FRAME_READ) schedule();
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
    if (n && !r && !(t & REACTIVE_ZOMBIE) && e._parent !== null && !(e._parent.R & REACTIVE_DISPOSED)) {
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
    const r = n.toString(36), t = r.length - 1;
    return e + (t ? String.fromCharCode(64 + t) : "") + r;
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
    const t = {
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
    if (n) linkChild(n, t);
    return t;
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
    for (let t = 0; t < e.length; t++) {
      const l = e[t];
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
      const t = this.un = [[], []];
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
        this.un[e2] = s || t[e2].length || this.un[e2].length ? t[e2].concat(s ? [] : o[e2], this.un[e2]) : o[e2];
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
    notify(e, n, o, t) {
      if (catchHook?.(e, o & n, t)) return true;
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
    const t = e.l & CONFIG_STAGED_ERROR;
    if (!o && n.T) n.T.de = void 0;
    e.l = o ? e.l & -1048833 | CONFIG_INPUTS_PUBLISHED : e.l & -3147137;
    if (t) {
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
  function actualInsertIntoHeap(e, t) {
    const E = (e._parent?.Qe ? e._parent.Ye?.Fe : e._parent?.Fe) ?? -1;
    if (E >= e.Fe) e.Fe = E + 1;
    const n = e.Fe;
    const I = t.$e[n];
    if (I === void 0) t.$e[n] = e;
    else {
      const t2 = I.We;
      t2.be = e;
      e.We = t2;
      I.We = e;
    }
    if (n > t.et) t.et = n;
  }
  function insertIntoHeap(e, t) {
    let E = e.R;
    if (E & (REACTIVE_IN_HEAP | REACTIVE_RECOMPUTING_DEPS)) return;
    if (E & REACTIVE_CHECK) {
      e.R = E & -4 | REACTIVE_DIRTY | REACTIVE_IN_HEAP;
    } else {
      e.R = E | REACTIVE_IN_HEAP;
      if (t.p) markNode(e);
    }
    if (!(E & REACTIVE_IN_HEAP_HEIGHT)) actualInsertIntoHeap(e, t);
  }
  function insertIntoHeapHeight(e, t) {
    let E = e.R;
    if (E & (REACTIVE_IN_HEAP | REACTIVE_RECOMPUTING_DEPS | REACTIVE_IN_HEAP_HEIGHT)) return;
    e.R = E | REACTIVE_IN_HEAP_HEIGHT;
    actualInsertIntoHeap(e, t);
  }
  function deleteFromHeap(e, t) {
    const E = e.R;
    if (!(E & (REACTIVE_IN_HEAP | REACTIVE_IN_HEAP_HEIGHT))) return;
    e.R = E & -25;
    const n = e.Fe;
    if (e.We === e) t.$e[n] = void 0;
    else {
      const E2 = e.be;
      const I = t.$e[n];
      const i = E2 ?? I;
      if (e === I) t.$e[n] = E2;
      else e.We.be = E2;
      i.We = e.We;
    }
    e.We = e;
    e.be = void 0;
  }
  function markHeap(e) {
    if (e.p) return;
    e.p = true;
    for (let t = 0; t <= e.et; t++) {
      for (let E = e.$e[t]; E !== void 0; E = E.be) {
        if (E.R & REACTIVE_IN_HEAP) markNode(E);
      }
    }
  }
  function markNode(e, t = REACTIVE_DIRTY) {
    const E = e.R;
    if ((E & (REACTIVE_CHECK | REACTIVE_DIRTY)) >= t) return;
    e.R = E & -4 | t;
    for (let t2 = e.o; t2 !== null; t2 = t2.t) {
      markNode(t2.u, REACTIVE_CHECK);
    }
  }
  function runHeap(e, t) {
    e.p = false;
    for (e.Ge = 0; e.Ge <= e.et; e.Ge++) {
      let E = e.$e[e.Ge];
      while (E !== void 0) {
        if (E.R & REACTIVE_IN_HEAP) t(E);
        else adjustHeight(E, e);
        E = e.$e[e.Ge];
      }
    }
    e.et = 0;
  }
  function adjustHeight(e, t) {
    deleteFromHeap(e, t);
    let E = e.Fe;
    for (let t2 = e.C; t2; t2 = t2.G) {
      const e2 = t2.S;
      if (e2.X && e2.Fe >= E) E = e2.Fe + 1;
    }
    if (e.Fe !== E) {
      e.Fe = E;
      for (let E2 = e.o; E2 !== null; E2 = E2.t) insertIntoHeapHeight(E2.u, t);
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
    const t = n._ = e.ve = {
      S: e,
      u: n,
      G: o,
      Xe: r,
      t: null,
      Te: n.Ne
    };
    if (l !== null) l.G = t;
    else n.C = t;
    if (r !== null) r.t = t;
    else e.o = t;
    bumpNotifyEpoch();
  }

  // node_modules/.pnpm/@solidjs+signals@2.0.0-rc.17/node_modules/@solidjs/signals/dist/prod/core/async.js
  function addPendingSource(e, t) {
    if (e.T?.fe?.has(t)) return false;
    (ext(e).fe ??= /* @__PURE__ */ new Set()).add(t);
    return true;
  }
  function removePendingSource(e, t) {
    const n = e.T?.fe;
    if (!n?.delete(t)) return false;
    if (!n.size) e.T.fe = void 0;
    return true;
  }
  function clearPendingSources(e) {
    if (e.T !== null) e.T.fe = void 0;
  }
  function retryReaches(e, t) {
    for (let n = e.C; n; n = n.G) {
      const e2 = n.S;
      if (e2 === t || e2.T?.fe?.has(t)) return true;
    }
    return false;
  }
  function parkLoadingWindow(e, t) {
    ext(e).ue = true;
    if (t.source) addPendingSource(e, t.source);
    if (!(e.m & STATUS_ERROR)) setPendingError(e, t.source, t);
  }
  function setPendingError(e, t, n) {
    if (!t) {
      if (e.T !== null) e.T.F = null;
      return;
    }
    if (n instanceof NotReadyError && n.source === t) {
      ext(e).F = n;
      return;
    }
    const r = e.T?.F;
    if (!(r instanceof NotReadyError) || r.source !== t) {
      ext(e).F = new NotReadyError(t);
    }
  }
  function forEachDependent(e, t) {
    for (let n = e.o; n !== null; n = n.t) t(n.u, n);
  }
  function releaseIfSettledUnobserved(e) {
    e.X && e.l & CONFIG_AUTO_DISPOSE && !e.o && !(e.R & REACTIVE_ZOMBIE) && !(e.m & STATUS_PENDING) && unobserved(e);
  }
  function releaseSettledDependents(e) {
    let t;
    const n = /* @__PURE__ */ new Set();
    const visit = (e2) => {
      if (n.has(e2)) return;
      n.add(e2);
      if (!e2.o && e2.l & CONFIG_AUTO_DISPOSE) (t ??= []).push(e2);
      forEachDependent(e2, visit);
    };
    forEachDependent(e, visit);
    if (t) for (const e2 of t) releaseIfSettledUnobserved(e2);
  }
  function stageError(e, t, n) {
    if (!(e.l & CONFIG_HELD || n && e.R & REACTIVE_STAGED_READ && globalQueue.q && !e.fam) || passLane) return false;
    clearPendingSources(e);
    if (e.T) e.T.ue = false;
    if (!((e.m &= STATUS_ERROR | STATUS_UNINITIALIZED) & STATUS_ERROR)) setPendingError(e);
    e.se = t instanceof StatusError ? t : new StatusError(e, t);
    e.l |= CONFIG_STAGED | CONFIG_STAGED_ERROR;
    settlePendingSource(e);
    schedule();
    return true;
  }
  function settleErroredDependents(e, t) {
    let n = false;
    const r = /* @__PURE__ */ new Set();
    const visit = (e2) => {
      if (r.has(e2)) return;
      r.add(e2);
      if (e2.T?.F === t) {
        enqueueSub(e2);
        n = true;
      }
      forEachDependent(e2, visit);
    };
    forEachDependent(e, visit);
    if (n) schedule();
  }
  function settlePendingSource(e, t = e) {
    removePendingSource(e, t);
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
      t !== e && retryReaches(f, t) || !removePendingSource(f, t))
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
    const t = e.T?.ae;
    if (t != null) {
      e.T.ae = null;
      t();
    }
  }
  function handleAsync(e, t, n, r) {
    let o = false;
    let f = false;
    if (typeof t === "object" && t !== null) {
      untrack(() => {
        o = t[Symbol.asyncIterator];
        f = !o && isThenable(t);
      });
    }
    if (!f && !o) {
      if (e.T !== null) e.T.Se = null;
      e.ce = false;
      return t;
    }
    ext(e).Se = t;
    e.T.Ee = question || MAINLINE_QUESTION;
    e.T.fe = void 0;
    let u;
    const handleError = (o2) => {
      if (e.T?.Se !== t) return;
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
      if (e.T?.Se !== t) return;
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
          } else if (e.T?.Se !== t) {
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
          } else if (e.T?.Se === t) {
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
    const flattenIfIterable = (e2, t2) => {
      let n2 = false;
      if (typeof e2 === "object" && e2 !== null) {
        untrack(() => {
          n2 = e2[Symbol.asyncIterator];
        });
      }
      if (!n2) return false;
      u = void 0;
      const r2 = consumeIterator(e2, t2);
      if (!t2) s = r2;
      return true;
    };
    if (f) {
      let n2 = false, r2 = false, o2, f2 = true;
      t.then((r3) => {
        if (f2) {
          u = r3;
          n2 = true;
        } else {
          try {
            if (e.T?.Se === t && !(e.R & REACTIVE_DISPOSED) && flattenIfIterable(r3, e)) return;
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
    if (o) flattenIfIterable(t);
    if (s !== null) {
      if (!s) {
        if (e.ce) return e.Ie;
        throw new NotReadyError(context);
      }
      e.ce = false;
    }
    return u;
  }
  function clearStatus(e, t = false) {
    if (e.T?.fe) clearPendingSources(e);
    if (e.T?.ue) {
      if (e.T !== null) e.T.ue = false;
    }
    e.m = t ? 0 : e.m & STATUS_UNINITIALIZED;
    if (e.T?.F) setPendingError(e);
    const n = statusNotifierOf(e);
    if (n) n.call(e);
  }
  function landStatus(e, t = false) {
    const n = e.T?.fe;
    if (n && (n.delete(e), n.size)) {
      e.T.ue = false;
      if (t) e.m = STATUS_PENDING;
      setPendingError(e, n.values().next().value);
    } else clearStatus(e, t);
  }
  function notifyStatus(e, t, n, r) {
    if (t === STATUS_ERROR && !(n instanceof StatusError) && !(n instanceof NotReadyError)) n = new StatusError(e, n);
    const o = t === STATUS_PENDING && n instanceof NotReadyError ? n.source : void 0;
    {
      if (t === STATUS_PENDING && o) {
        if (e.m & STATUS_ERROR) e.T.de = e.T.F;
        addPendingSource(e, o);
        e.m = STATUS_PENDING | e.m & STATUS_UNINITIALIZED;
        setPendingError(e, o, n);
      } else {
        clearPendingSources(e);
        e.m = t | (t !== STATUS_ERROR ? e.m & STATUS_UNINITIALIZED : 0);
        ext(e).F = n;
      }
    }
    if (t === STATUS_PENDING && e.l & CONFIG_GUESS) {
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
    propagateStatus(e, t, n);
  }
  function propagateStatus(e, t, n, r) {
    const o = t === STATUS_PENDING && n instanceof NotReadyError ? n.source : void 0;
    forEachDependent(e, (r2, f) => {
      r2.ie = clock;
      if (r2.l & (CONFIG_REDERIVE | CONFIG_VERDICT) || t === STATUS_PENDING && f.Te !== r2.Ne) {
        enqueueSub(r2);
        schedule();
        return;
      }
      if (t === STATUS_PENDING && o && r2.T?.fe?.has(o)) {
        if (passLane === null && (r2.l & (CONFIG_HELD | CONFIG_OVERRIDE)) === CONFIG_HELD && globalQueue.q && !freshHook?.(r2, o.ie === clock)) joinFuture(txOf(r2));
        return;
      }
      if (t === STATUS_PENDING && o || t !== STATUS_PENDING && (r2.T?.F !== n || r2.T?.fe)) {
        {
          if (laneStageHook && passLane !== null) laneStageHook(r2, passLane, true, e);
          else {
            queuePendingNode(r2);
            if (t === STATUS_PENDING && (r2.l & (CONFIG_HELD | CONFIG_OVERRIDE)) === CONFIG_HELD && !r2.i) joinFuture(txOf(r2));
          }
        }
        notifyStatus(r2, t, n);
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
    let t = e._parent;
    while (t.R & REACTIVE_ZOMBIE) t = t._parent;
    return (t.l & (CONFIG_OVERRIDE | CONFIG_GUESS)) === CONFIG_OVERRIDE || t.i && t.T?.Re?.v;
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
  function recompute(e, t = false) {
    bumpNotifyEpoch();
    const n = e.i;
    let o;
    const E = passLane;
    const s = laneRoot;
    const i = laneStageHook && (laneRoot = t && (E || s));
    setPassLane(!laneStageHook ? null : (e.R & REACTIVE_LANE_DIRTY || (e.l & (CONFIG_OVERRIDE | CONFIG_GUESS)) === CONFIG_OVERRIDE) && e.T?.Re?.v ? e.T.Re : (
      // Loose `!=`: false with no creator lane (undefined == null).
      t && E?._parent.he != E && creatorPass(context)?.R & REACTIVE_RECOMPUTING_DEPS ? E : null
    ));
    if (!t) {
      deleteFromHeap(e, dirtyQueue);
      if (e.T !== null) {
        e.T.Se = null;
        releaseFlightTeardown(e);
      }
      if (e.m & STATUS_PENDING) e.l &= ~CONFIG_VERDICT;
      if (e.l & CONFIG_HELD) {
        const t2 = txOf(e);
        if (t2.v || e.l & CONFIG_LAGS) ;
        else if (n !== EFFECT_RENDER && !(e.l & CONFIG_VERDICT)) joinPassTx(t2, freshHook?.(e, e.m & STATUS_UNINITIALIZED));
        else if (t2 !== flushTransaction && !(e.m & STATUS_UNINITIALIZED || e.l & CONFIG_HELD_FRAME)) {
          e.se = NOT_PENDING;
          if (n === EFFECT_RENDER) e.l &= ~CONFIG_HELD;
        }
      }
      if (n === EFFECT_TRACKED || e.l & CONFIG_STAGED) {
        const t2 = deferredReleases;
        deferReleases(o = []);
        disposeChildren(e);
        deferReleases(t2);
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
        const t2 = e.T?.Se;
        const n2 = e.X(T);
        const o2 = typeof n2 === "object" && n2 !== null;
        const E2 = e.T?.Se !== t2;
        T = E2 || !o2 ? n2 : handleAsync(e, n2);
        if (!E2 && !o2) {
          if (e.T !== null) e.T.Se = null;
          e.ce = false;
        }
      }
      if (e.m !== 0 || e.T !== null) clearStatus(e, t);
    } catch (t2) {
      const n2 = t2 instanceof NotReadyError;
      if (n2 && e.ce) {
        parkLoadingWindow(e, t2);
      } else {
        if (n2 || !(I = stageError(e, t2, true))) {
          if (n2) ext(e).ue = true;
          notifyStatus(e, n2 ? STATUS_PENDING : STATUS_ERROR, t2);
          if (n2 && a && !e.T?.Se) settlePendingSource(e);
        }
        if ((n2 || I) && _) {
          for (const t3 of _) if (t3 !== e && !e.T?.fe?.has(t3)) settlePendingSource(e, t3);
        }
      }
    } finally {
      tracking = R;
      if (laneStageHook) laneRoot = s;
      c = (e.R & REACTIVE_MISSED_WAKE) !== 0;
      A = !!(e.R & (i && (E || n || !(e.m & STATUS_PENDING)) ? REACTIVE_JOINED | REACTIVE_STAGED_READ : REACTIVE_JOINED));
      e.l = e.l & ~CONFIG_VERDICT | e.R & REACTIVE_PROBED;
      e.R = e.R & (REACTIVE_ZOMBIE | REACTIVE_DISPOSED | REACTIVE_FRAME_READ | REACTIVE_STAGED_READ | REACTIVE_LANE_READ | REACTIVE_SCREEN_READ | REACTIVE_PROBE_UNANSWERED) | (t ? e.R & REACTIVE_SNAPSHOT_STALE : 0);
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
      if (laneStageHook(e, C, t, d)) setPassLane(C);
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
        if (!t || C !== null) globalQueue.enqueue(n, e.W ??= GlobalQueue.K.bind(null, e), C);
      }
      if (!t && C === null && e.l & CONFIG_GUESS && laneOutcomeHook(e, T, d)) ;
      else if (d) ;
      else if (E2) {
        if (C !== null) {
          if (n && e.se === NOT_PENDING) e.Ie = T;
          else if (n || C.ke) e.se = T;
          else e.T.v = T;
        } else if (t ? !((flushTransaction !== null || passTx !== null) && (A || creatorPass(f)?.R & REACTIVE_JOINED)) : n && e.se === NOT_PENDING) {
          e.Ie = T;
        } else {
          e.se = T;
          if (t) {
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
        for (let t2 = e.o; t2 !== null; t2 = t2.t) insertIntoHeapHeight(t2.u, dirtyQueue);
      }
      if (!E2 && !e.T?.F) {
        if (r !== void 0) settleErroredDependents(e, r);
        if (_) {
          for (const t2 of _) if (t2 !== e) settlePendingSource(e, t2);
        }
      }
      if (a && !(e.m & (STATUS_PENDING | STATUS_UNINITIALIZED))) settlePendingSource(e);
    }
    if (t && C === null && A && e.m & STATUS_PENDING) {
      const t2 = flushTransaction ?? passTx;
      if (t2 && !t2.v && freshHook?.(e) !== CAUGHT_REARMED) holdNode(e, t2);
    }
    if (!e.T?.F && e.se === NOT_PENDING && !(n && e.Ae)) {
      if (t || n === EFFECT_TRACKED) trimStaleDeps(e);
      else heldTrim(e);
    }
    if (e.se !== NOT_PENDING || e.T !== null && (e.T.Ue !== null || e.T.me !== null) || (e.m & (STATUS_PENDING | STATUS_UNINITIALIZED)) !== 0 && (!t || (e.m & STATUS_PENDING) !== 0)) {
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
      for (let t = e.C; t; t = t.G) {
        const n = t.S;
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
  function computed(e, t) {
    const n = t?.transparent ?? false;
    const o = t !== null && typeof t === "object" && "loadingValue" in t;
    const E = {
      id: inheritId(t, n, context),
      l: (n ? CONFIG_TRANSPARENT : 0) | (t?.ownedWrite ? CONFIG_OWNED_WRITE : 0) | (!context || t?.lazy ? CONFIG_AUTO_DISPOSE : 0) | (t?.sync ? CONFIG_SYNC : 0) | (t?.le ? CONFIG_NO_SNAPSHOT : 0) | (t?.Le ?? 0) | (snapshotCaptureActive && ownerInSnapshotScope(context) ? CONFIG_IN_SNAPSHOT_SCOPE : 0),
      xe: t?.equals ?? isEqual,
      He: null,
      A: context?.A ?? defaultContext,
      ye: 0,
      X: e,
      Ie: o ? t.loadingValue : void 0,
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
      R: t?.lazy ? REACTIVE_LAZY : REACTIVE_NONE,
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
    if (t?.unobserved) ext(E).we = t.unobserved;
    setupComputedNode(E, t);
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
  function joinPass(e, t) {
    e.R |= REACTIVE_JOINED;
    if (t.m & STATUS_UNINITIALIZED || e.i !== EFFECT_RENDER) joinPassTx(txOf(t), !e.i && e.m & STATUS_UNINITIALIZED || freshHook?.(e));
  }
  function frameRead(e, t) {
    const n = observeFlightHook && e.l & CONFIG_VERDICT;
    const o = laneStageHook && passLane !== null;
    if ((!o || e.m & STATUS_UNINITIALIZED && t.m & STATUS_PENDING) && !n && (e.i !== EFFECT_RENDER || t.l & CONFIG_INPUTS_PUBLISHED)) return false;
    const E = txOf(t);
    if (E === flushTransaction && !o && !n || bornInto(e, E)) return false;
    staleReader(e, E);
    return true;
  }
  function bornInto(e, t) {
    return (e.m & STATUS_UNINITIALIZED || e.l & CONFIG_HELD_FRAME) && e.l & CONFIG_HELD && txOf(e) === t;
  }
  function creatorPass(e) {
    return e !== null && e.Qe ? e.Ye : e;
  }
  function parkChildren(e) {
    markDisposal(e);
    const t = ext(e);
    t.me = e.He;
    t.Ue = e.De;
    e.He = null;
    e.De = null;
    e.ye = 0;
  }
  function createEffectNode(e, t, n, o, E) {
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
      Ke: t,
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
  function setupComputedNode(e, t) {
    e.We = e;
    const n = context?.Qe ? context.Ye : context;
    if (context) linkChild(context, e);
    if (n) e.Fe = n.Fe + 1;
    wireExternalSourceHook?.(e);
    !t?.lazy && recompute(e, true);
    if (snapshotCaptureActive && !t?.lazy) {
      if (!(e.m & STATUS_PENDING) && !(e.l & CONFIG_NO_SNAPSHOT)) {
        ext(e).Pe = e.Ie === void 0 ? NO_SNAPSHOT : e.Ie;
        e.l |= CONFIG_HAS_SNAPSHOT;
        snapshotSources.add(e);
      }
    }
  }
  function signal(e, t) {
    const n = {
      xe: t?.equals ?? isEqual,
      l: (t?.ownedWrite ? CONFIG_OWNED_WRITE : 0) | (t?.le ? CONFIG_NO_SNAPSHOT : 0),
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
    if (t?.unobserved) ext(n).we = t.unobserved;
    if (snapshotCaptureActive && !(n.l & CONFIG_NO_SNAPSHOT)) {
      ext(n).Pe = e === void 0 ? NO_SNAPSHOT : e;
      n.l |= CONFIG_HAS_SNAPSHOT;
      snapshotSources.add(n);
    }
    return n;
  }
  function isEqual(e, t) {
    return e === t;
  }
  var verdict = null;
  function untrack(e, t) {
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
  function prepareComputed(e, t) {
    if (e.R & REACTIVE_LAZY) {
      e.R &= ~REACTIVE_LAZY;
      recompute(e, true);
    } else if (e.R & REACTIVE_DISPOSED) {
      if (e.l & CONFIG_AUTO_DISPOSE) {
        const t2 = e._parent;
        if (t2 !== null) {
          if (t2.R & REACTIVE_DISPOSED) {
            e.l &= ~CONFIG_AUTO_DISPOSE;
            return;
          }
          if (!(e.R & REACTIVE_ZOMBIE)) linkChild(t2, e);
        }
        recompute(e, true);
      }
    } else ;
  }
  function readerSeesCommitted(e, t) {
    return !t || e.se === NOT_PENDING || t.l & CONFIG_CHILDREN_FORBIDDEN;
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
    const t = e.T;
    if (t.Be !== clock) {
      t.je = e.se;
      t.Be = clock;
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
  function pullComputed(e, t) {
    if (e.Fe >= dirtyQueue.Ge) {
      markNode(t);
      markHeap(dirtyQueue);
      updateIfNecessary(e);
    } else if (t.l & CONFIG_FRESH_READ) updateIfNecessary(e);
    const n = e.Fe;
    if (n >= t.Fe && e._parent !== t) t.Fe = n + 1;
  }
  function read(e) {
    let t = context;
    if (t?.Qe) t = t.Ye;
    const n = e;
    const o = e;
    if (typeof n.X === "function") prepareComputed(e);
    if (verdict !== null) return verdict(e, t);
    if (!n.X && e.T?.Pe === void 0 && !snapshotCaptureActive && (!unflushedStaged || e.se === NOT_PENDING) && // A28, see readNodeFast
    !(e.l & CONFIG_OVERRIDE) && // lanes: the slow path's one arm
    true) {
      if (t) {
        if (tracking) link(e, t);
        if (e.l & CONFIG_HELD && !(t.l & CONFIG_CHILDREN_FORBIDDEN)) {
          if (frameRead(t, e)) return e.Ie;
          joinPass(t, e);
        }
      }
      if (!t || e.se === NOT_PENDING || t.l & CONFIG_CHILDREN_FORBIDDEN) return e.Ie;
      if ((observeFlightHook && t.l & CONFIG_VERDICT || laneStageHook && passLane?.folded) && stagedScreen(t)) return e.Ie;
      stagedRead(t);
      return e.se;
    }
    let E = false;
    if (t) {
      if (tracking) {
        link(e, t);
        if (o.X) {
          pullComputed(o, t);
          if (!t.i) t.R |= o.l & CONFIG_VERDICT;
        }
      }
      if (e.l & CONFIG_HELD && !(e.l & CONFIG_OVERRIDE) && !(t.l & CONFIG_CHILDREN_FORBIDDEN)) {
        if (o.m & STATUS_UNINITIALIZED || !frameRead(t, e)) joinPass(t, e);
        else E = true;
      }
    }
    if (laneReadHook && e.l & CONFIG_OVERRIDE) {
      const n2 = laneReadHook(t, e);
      if (n2 !== NOT_PENDING) return n2;
    }
    if (o.m & STATUS_PENDING && !E && !(e.l & CONFIG_OVERRIDE) && !(o.m & STATUS_UNINITIALIZED)) {
      if (laneStageHook && passLane !== null && !(t?.m & STATUS_UNINITIALIZED)) E = true;
      else if (observeFlightHook && t !== null && t.l & CONFIG_VERDICT) E = observeFlightHook(t, o);
    }
    if (o.m & STATUS_PENDING && !E) {
      if (t) {
        if (!(t.l & CONFIG_CHILDREN_FORBIDDEN) || o.m & STATUS_UNINITIALIZED) {
          if (!tracking && !spectating && e !== t) link(e, t);
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
    if (snapshotCaptureActive && t && t.l & CONFIG_IN_SNAPSHOT_SCOPE) {
      const n2 = e.T?.Pe;
      if (n2 !== void 0) {
        const o2 = n2 === NO_SNAPSHOT ? void 0 : n2;
        const E2 = e.se !== NOT_PENDING ? e.se : e.Ie;
        if (E2 !== o2) t.R |= REACTIVE_SNAPSHOT_STALE;
        return o2;
      }
    }
    if (E) return e.Ie;
    if (!t && e.T?.de) throw e.T.de;
    const s = serve(e, t);
    if (!t && typeof n.X === "function" && e.l & CONFIG_AUTO_DISPOSE && !(o.m & STATUS_PENDING) && !e.o) {
      dormantNodes.add(e);
      schedule();
    }
    return s;
  }
  function serve(e, t) {
    if (e.se !== NOT_PENDING && e.m & STATUS_UNINITIALIZED && (!t || spectating || t.l & CONFIG_CHILDREN_FORBIDDEN)) throw new NotReadyError(null);
    const n = t && unflushedStaged ? unflushedValue(e) : NOT_PENDING;
    if (n !== NOT_PENDING) {
      markLateLinker(t);
      return n;
    }
    if (readerSeesCommitted(e, t)) return e.Ie;
    if (observeFlightHook && t.l & CONFIG_VERDICT && !(e.m & STATUS_UNINITIALIZED) && stagedScreen(t)) return e.Ie;
    stagedRead(t, e);
    if (e.l & CONFIG_STAGED_ERROR) throw e.se;
    return e.se;
  }
  function stagedRead(e, t) {
    if (!(t?.m & STATUS_UNINITIALIZED)) {
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
  function captureWriteSnapshot(e, t) {
    if (e.l & CONFIG_NO_SNAPSHOT || e.X !== void 0 || e.T?.Pe !== void 0) return;
    ext(e).Pe = t === void 0 ? NO_SNAPSHOT : t;
    e.l |= CONFIG_HAS_SNAPSHOT;
    snapshotSources.add(e);
  }
  var writeHook = null;
  function isSame(e, t, n) {
    if (e.xe === isEqual) return t === n;
    try {
      return e.xe && e.xe(t, n);
    } catch (t2) {
      if (!(t2 instanceof NotReadyError)) return notifyStatus(e, STATUS_ERROR, t2), true;
    }
  }
  function setSignal(e, t) {
    if (writeHook !== null) writeHook(e);
    const n = e.se === NOT_PENDING ? e.Ie : e.se;
    if (typeof t === "function") t = t(n);
    if (laneWriteHook && e.l & CONFIG_OVERRIDE) return laneWriteHook(e, t);
    if (e.l & CONFIG_HELD) joinFuture(txOf(e));
    const o = !!(e.m & STATUS_UNINITIALIZED) || !!e.T?.de || !isSame(e, n, t);
    if (!o) return t;
    if (snapshotCaptureActive) captureWriteSnapshot(e, n);
    const E = e.se !== NOT_PENDING;
    if (!E) queuePendingNode(e);
    else if (e.l & CONFIG_HELD && !globalQueue.q) stashFlushed(e);
    e.se = t;
    if (context !== null) notePromotedWrite(e);
    if (e.X !== void 0) e.ie = clock;
    if (E && e.Me === notifyEpoch) return t;
    insertSubs(e);
    schedule();
    return t;
  }
  function setMemo(e, t) {
    const n = e.l & CONFIG_HELD && !(e.R & REACTIVE_MANUAL_WRITE) && txOf(e) !== flushTransaction;
    if (n && typeof t === "function") {
      const n2 = t(e.Ie);
      t = () => n2;
    }
    if (e.m & STATUS_ERROR) {
      e.T.de = e.T.F;
      notifyStatus(e, STATUS_NONE, void 0);
    }
    t = setSignal(e, t);
    e.l &= ~CONFIG_STAGED_ERROR;
    if (n) {
      e.R = e.R & ~REACTIVE_CHECK | REACTIVE_DIRTY;
      insertIntoHeap(e, dirtyQueue);
      schedule();
    } else if (e.se !== NOT_PENDING) e.R |= REACTIVE_MANUAL_WRITE;
    return t;
  }
  function runWithOwner(e, t) {
    const n = context;
    const o = tracking;
    context = e;
    tracking = false;
    try {
      return t();
    } finally {
      context = n;
      tracking = o;
    }
  }

  // node_modules/.pnpm/@solidjs+signals@2.0.0-rc.17/node_modules/@solidjs/signals/dist/prod/core/identities.js
  var signalHook = null;

  // node_modules/.pnpm/@solidjs+signals@2.0.0-rc.17/node_modules/@solidjs/signals/dist/prod/core/effect.js
  function effect(t, e, E, u) {
    const f = !!u?.user;
    const r = createEffectNode(t, e, E, f ? EFFECT_USER : EFFECT_RENDER, u);
    recompute(r, true);
    !u?.defer && r.se === NOT_PENDING && !r.T?.Re?.v && (r.i === EFFECT_USER || u?.schedule ? globalQueue.enqueue(r.i, runEffect.bind(null, r)) : runEffect(r, r.i));
  }
  function notifyEffectStatus(t, e) {
    const E = t !== void 0 ? t : this.m;
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
  function runEffect(t, e) {
    if (!t.Ae || t.R & REACTIVE_DISPOSED) return;
    if (heldRunHook?.(t)) return;
    if (t.m & STATUS_ERROR && t.i === EFFECT_USER) {
      const e2 = unwrapStatusError(t.T?.F);
      t.ge = t.Ie;
      t.Ae = false;
      try {
        t.qe ? t.qe(e2, () => {
          const e3 = t.Je;
          t.Je = void 0;
          e3?.();
        }) : console.error(e2);
      } catch (e3) {
        if (!globalQueue.notify(t, STATUS_ERROR, STATUS_ERROR)) {
          haltReactivity(e3);
          throw e3;
        }
      }
      return;
    }
    const E = t.T?.F == null;
    const u = t.Je;
    t.Je = void 0;
    try {
      u?.();
      const e2 = t.Ke(t.Ie, t.ge);
      if (false) ;
      t.Je = e2;
    } catch (e2) {
      ext(t).F = new StatusError(t, e2);
      t.m |= STATUS_ERROR;
      if (!globalQueue.notify(t, STATUS_ERROR, STATUS_ERROR)) {
        haltReactivity(e2);
        throw e2;
      }
    } finally {
      t.ge = t.Ie;
      t.Ae = false;
      if (E) trimStaleDeps(t);
    }
  }
  GlobalQueue.K = runEffect;
  setEffectStatusNotify(notifyEffectStatus);

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
    const t = read.bind(null, e);
    t[$REFRESH] = e;
    return t;
  }
  function createSignal(e, t) {
    if (typeof e === "function") {
      const r2 = computed(e, t);
      r2.l &= ~CONFIG_AUTO_DISPOSE;
      return [accessor(r2), setMemo.bind(null, r2)];
    }
    const r = signal(e, t);
    if (signalHook !== null) signalHook(r);
    return [accessor(r), setSignal.bind(null, r)];
  }
  function createMemo(e, t) {
    return accessor(computed(e, t));
  }
  function createRenderEffect(e, t, r) {
    effect(e, t, void 0, r);
  }

  // node_modules/.pnpm/@solidjs+signals@2.0.0-rc.17/node_modules/@solidjs/signals/dist/prod/map.js
  function mapArray(t, e, i) {
    const s = typeof i?.keyed === "function" ? i.keyed : void 0;
    const n = e.length > 1;
    const r = e;
    const o = {
      L: createOwner(),
      it: 0,
      st: t,
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
  function trySmallMove(t, e, i, s) {
    const n = t.ot;
    const r = t.it - 1;
    const o = [];
    const f = [];
    const h = [];
    let c = 256;
    let a = s;
    let l = s;
    let u = false;
    while (a <= r && l <= i - 1) {
      const t2 = n[a];
      const s2 = e[l];
      if (t2 === s2) {
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
      for (let t3 = 1; t3 <= w; t3++) {
        if (n[a + t3] === s2) {
          d = t3;
          break;
        }
      }
      c -= d === -1 ? w : d;
      let p = -1;
      w = Math.min(32 - f.length, i - 1 - l, c);
      for (let i2 = 1; i2 <= w; i2++) {
        if (e[l + i2] === t2) {
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
    return commitSmallMove(t, e, i, o, f, h);
  }
  function commitSmallMove(t, e, i, s, n, r) {
    const o = t.ot;
    let f;
    let h;
    let c;
    if (n.length !== 0) {
      c = new Array(s.length);
      for (h = 0; h < n.length; h++) {
        let t2 = -1;
        for (f = 0; f < s.length; f++) {
          if (!c[f] && o[s[f]] === e[n[h]]) {
            t2 = f;
            break;
          }
        }
        if (t2 === -1) return false;
        c[t2] = true;
        n[h] = n[h] << 6 | t2;
      }
    }
    if (s.length !== 0 || n.length !== 0) {
      const t2 = /* @__PURE__ */ new Set();
      for (f = 0; f < s.length; f++) t2.add(o[s[f]]);
      for (h = 0; h < n.length; h++) t2.add(e[n[h] >> 6]);
      for (let e2 = 0; e2 < r.length; e2 += 3) {
        const i2 = r[e2];
        for (let s2 = 0, n2 = r[e2 + 2]; s2 < n2; s2++) if (t2.has(o[i2 + s2])) return false;
      }
    }
    const a = t.ht;
    const l = t.tn;
    const u = a.slice(0, i);
    const d = l.slice(0, i);
    for (let t2 = 0; t2 < r.length; t2 += 3) {
      const e2 = r[t2];
      const i2 = r[t2 + 1];
      if (e2 !== i2) {
        for (let s2 = 0; s2 < r[t2 + 2]; s2++) {
          u[i2 + s2] = a[e2 + s2];
          d[i2 + s2] = l[e2 + s2];
        }
      }
    }
    for (h = 0; h < n.length; h++) {
      const t2 = n[h] >> 6;
      const e2 = s[n[h] & 63];
      u[t2] = a[e2];
      d[t2] = l[e2];
    }
    t.ht = u;
    t.tn = d;
    t.it = i;
    t.ot = e.slice(0);
    for (f = 0; f < s.length; f++) {
      if (c === void 0 || !c[f]) l[s[f]].dispose();
    }
    return true;
  }
  function stageRemoval(t, e, i, s, n, r, o, f, h) {
    const c = t.ze;
    const a = t.ot;
    const l = t.tn;
    const u = [];
    let d = i;
    for (let w = i; w <= n; d++, w++) {
      const n2 = c ? c(e[w]) : e[w];
      while (d <= s && (c ? c(a[d]) : a[d]) !== n2) u.push(l[d++]);
      if (d > s) {
        o.fill(void 0);
        return null;
      }
      r[w] = t.ht[d];
      o[w - i] = l[d];
      if (f) f[w - i] = t.ct[d];
      if (h) h[w - i] = t.lt[d];
    }
    for (; d <= s; d++) u.push(l[d]);
    return u;
  }
  function commitWindow(t, e, i, s, n, r, o, f, h) {
    const c = t.tn;
    const a = t.ct;
    const l = t.lt;
    const u = t.it;
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
    const t = this.st() || [], e = t.length;
    t[$TRACK];
    runWithOwner(this.L, () => {
      let i, s, n, r, o, f = this.ct ? this.ut ? () => {
        r[n] = signal(t[s], pureOptions);
        return this.ft(accessor(r[n]), s);
      } : () => {
        r[n] = signal(t[s], pureOptions);
        o && (o[n] = signal(s, pureOptions));
        return this.ft(accessor(r[n]), o ? accessor(o[n]) : void 0);
      } : this.lt ? () => {
        const e2 = t[s];
        o[n] = signal(s, pureOptions);
        return this.ft(e2, accessor(o[n]));
      } : () => {
        const e2 = t[s];
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
        } catch (t2) {
          for (i = 0; i <= s; i++) c[i]?.dispose();
          throw t2;
        }
        if (this.tn[0]) this.tn[0].dispose();
        this.ht = h;
        this.tn = c;
        r && (this.ct = r);
        o && (this.lt = o);
        this.ot = t.slice(0);
        this.it = e;
      } else {
        let h, c, a, l, u, d, w, p, m, O, _;
        const y = setSignal;
        for (h = 0, c = Math.min(this.it, e); h < c && (this.ot[h] === t[h] || this.ct && compare(this.ze, this.ot[h], t[h])); h++) {
          if (this.ct) y(this.ct[h], t[h]);
        }
        for (c = this.it - 1, a = e - 1; c >= h && a >= h && (this.ot[c] === t[a] || this.ct && compare(this.ze, this.ot[c], t[a])); c--, a--) ;
        if (h === e && this.it === e) {
          this.ot = t.slice(0);
          return;
        }
        if (e <= this.it && c - h > 64 && this.ct === void 0 && this.lt === void 0) {
          const i2 = h + (a - h >> 1);
          const s2 = t[i2];
          const n2 = Math.min(c, i2 + 32);
          let r2 = Math.max(h, i2 - 32);
          while (r2 <= n2 && this.ot[r2] !== s2) r2++;
          if (r2 <= n2 && trySmallMove(this, t, e, h)) return;
        }
        const g = e - this.it;
        const A = a - h + 1;
        const S = new Array(e);
        const M = new Array(A);
        r = this.ct ? new Array(A) : void 0;
        o = this.lt ? new Array(A) : void 0;
        const k = g < 0 ? stageRemoval(this, t, h, c, a, S, M, r, o) : null;
        if (k !== null) {
          if (k.length !== 0) p = k;
        } else {
          d = /* @__PURE__ */ new Map();
          w = new Array(A);
          for (s = a; s >= h; s--) {
            l = t[s];
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
          } catch (t2) {
            if (m) for (i = 0; i < m.length; i++) m[i].dispose();
            throw t2;
          }
        }
        for (i = 0; i < h; i++) S[i] = this.ht[i];
        for (s = a + 1; s < e; s++) S[s] = this.ht[s - g];
        commitWindow(this, t, e, h, c, a, M, r, o);
        this.ht = S;
        this.it = e;
        this.ot = t.slice(0);
        if (p) for (i = 0; i < p.length; i++) p[i].dispose();
        if (false) ;
      }
    });
    return this.ht;
  }
  function compare(t, e, i) {
    return t ? t(e) === t(i) : true;
  }

  // node_modules/.pnpm/@solidjs+signals@2.0.0-rc.17/node_modules/@solidjs/signals/dist/prod/flatten.js
  function flatten(r, t) {
    if (typeof r === "function" && !r.length) {
      if (t?.doNotUnwrap) return r;
      do {
        r = r();
      } while (typeof r === "function" && !r.length);
    }
    if (t?.skipNonRendered && (r == null || r === true || r === false || r === "")) return;
    if (Array.isArray(r)) {
      let e = [];
      if (flattenArray(r, e, t)) {
        return () => {
          let r2 = [];
          flattenArray(e, r2, {
            ...t,
            doNotUnwrap: false
          });
          return r2;
        };
      }
      return e;
    }
    return r;
  }
  function flattenArray(r, t = [], e) {
    let o = null;
    let f = false;
    for (let n = 0; n < r.length; n++) {
      try {
        let o2 = r[n];
        if (typeof o2 === "function" && !o2.length) {
          if (e?.doNotUnwrap) {
            t.push(o2);
            f = true;
            continue;
          }
          do {
            o2 = o2();
          } while (typeof o2 === "function" && !o2.length);
        }
        if (Array.isArray(o2)) {
          f = flattenArray(o2, t, e) || f;
        } else if (e?.skipNonRendered && (o2 == null || o2 === true || o2 === false || o2 === "")) {
        } else t.push(o2);
      } catch (r2) {
        if (!(r2 instanceof NotReadyError)) throw r2;
        o = r2;
      }
    }
    if (o) throw o;
    return f;
  }

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
    constructor(e, n, t) {
      this.source = e;
      this.kind = n;
      this.hidden = t;
    }
  };
  function isHidden(e, n) {
    const t = e.hidden;
    return typeof t === "function" ? t(n) : t.includes(n);
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
      const t = [];
      for (let r = 0; r < n2.length; r++) if (!isHidden(e, n2[r])) t.push(n2[r]);
      return t;
    }
    return leafKeys(leafOf(e, n), n);
  }
  function sourceHas(e, n, t) {
    if (n === SOURCE_OMIT) {
      if (isHidden(e, t)) return false;
      return e.kind === SOURCE_MERGE ? mergeHas(e.source, t) : t in viewSource(e);
    }
    return t in leafOf(e, n);
  }
  function sourceGet(e, n, t) {
    if (n === SOURCE_OMIT) {
      if (isHidden(e, t)) return void 0;
      return e.kind === SOURCE_MERGE ? mergeGet(e.source, t) : viewSource(e)[t];
    }
    return leafOf(e, n)[t];
  }
  function entryHasStaticKeys(e, n) {
    if (n === SOURCE_PLAIN) return true;
    if (n !== SOURCE_OMIT) return false;
    if (e.kind === SOURCE_PLAIN) return true;
    return e.kind === SOURCE_MERGE && mergeHasStaticKeys(e.source);
  }
  function mergeHasStaticKeys(e) {
    const n = e.sources, t = e.kinds;
    for (let e2 = 0; e2 < n.length; e2++) if (!entryHasStaticKeys(n[e2], t[e2])) return false;
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
  function sourceDescriptor(e, n, t, r = false) {
    if (n === SOURCE_OMIT) {
      if (isHidden(e, t)) return void 0;
      return sourceDescriptor(e.source, e.kind, t, r);
    }
    if (n === SOURCE_MERGE) return mergeDescriptor(e, t);
    if (n === SOURCE_MEMO) {
      return r || t in leafOf(e, n) ? accessorDescriptor(() => leafOf(e, n)[t]) : void 0;
    }
    if (n === SOURCE_PROXY) {
      if (recordOf(e) !== void 0) return Reflect.getOwnPropertyDescriptor(e, t);
      return r || t in e ? accessorDescriptor(() => e[t]) : void 0;
    }
    const i = Reflect.getOwnPropertyDescriptor(e, t);
    if (i === void 0) return void 0;
    if (i.get !== void 0 || i.set !== void 0) return accessorDescriptor(() => e[t], i.enumerable);
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
      const t = e.sources, r = e.kinds;
      for (let n2 = 0; n2 < t.length; n2++) {
        if (!entryHasStaticKeys(t[n2], r[n2])) {
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
  function collectTable(e, n, t) {
    const r = n.sources, i = n.kinds;
    for (let n2 = 0; n2 < r.length; n2++) {
      const f = r[n2];
      if (i[n2] === SOURCE_OMIT) {
        if (f.kind === SOURCE_MERGE) {
          if (t === void 0) t = [f];
          else t.push(f);
          collectTable(e, f.source, t);
          t.pop();
          continue;
        }
        const n3 = f.source;
        const r2 = Reflect.ownKeys(n3);
        for (let i2 = 0; i2 < r2.length; i2++) {
          const u = r2[i2];
          if (!isHidden(f, u) && !hiddenByAny(t, u)) tableSet(e, u, n3);
        }
      } else {
        const n3 = Reflect.ownKeys(f);
        for (let r2 = 0; r2 < n3.length; r2++) {
          const i2 = n3[r2];
          if (!hiddenByAny(t, i2)) tableSet(e, i2, f);
        }
      }
    }
  }
  function hiddenByAny(e, n) {
    if (e !== void 0) {
      for (let t = e.length - 1; t >= 0; t--) if (isHidden(e[t], n)) return true;
    }
    return false;
  }
  function tableSet(e, n, t) {
    if (e.has(n)) e.delete(n);
    e.set(n, t);
  }
  function omitTable(e) {
    let n = e.table;
    if (typeof n !== "object") {
      const t = e.source;
      if (e.kind === SOURCE_MERGE) {
        if (!mergeHasStaticKeys(t)) {
          e.table = null;
          return void 0;
        }
        n = /* @__PURE__ */ new Map();
        collectTable(n, t, [e]);
      } else if (e.kind === SOURCE_PLAIN) {
        n = /* @__PURE__ */ new Map();
        const r = Reflect.ownKeys(t);
        for (let i = 0; i < r.length; i++) {
          const f = r[i];
          if (!isHidden(e, f)) n.set(f, t);
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
    let t = e.keys;
    if (t === void 0) {
      t = e.keys = [];
      for (const [e2, r] of n) if (propertyIsEnumerable.call(r, e2)) t.push(e2);
    }
    return t;
  }
  function tableDescriptor(e, n, t) {
    const r = n.get(t);
    if (r === void 0) return void 0;
    let i = e.descs;
    if (i === void 0) i = e.descs = /* @__PURE__ */ new Map();
    let f = i.get(t);
    if (f === void 0) {
      f = sourceDescriptor(r, SOURCE_PLAIN, t);
      if (f === void 0) return void 0;
      i.set(t, f);
      return f;
    }
    if (f.get !== void 0) return f;
    return {
      configurable: true,
      enumerable: f.enumerable,
      writable: f.writable,
      value: r[t]
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
    const t = tableOf(e);
    if (t !== void 0) {
      const e2 = t.get(n);
      return e2 === void 0 ? MISSING : e2[n];
    }
    const r = e.sources, i = e.kinds;
    for (let e2 = r.length - 1; e2 >= 0; e2--) {
      const t2 = i[e2];
      if (t2 === SOURCE_PLAIN) {
        const t3 = r[e2];
        if (n in t3) return t3[n];
        continue;
      }
      if (t2 === SOURCE_OMIT) {
        const t3 = r[e2];
        if (isHidden(t3, n)) continue;
        if (t3.kind === SOURCE_MERGE) {
          const e3 = mergeLookup(t3.source, n);
          if (e3 !== MISSING) return e3;
          continue;
        }
        const i2 = viewSource(t3);
        if (n in i2) return i2[n];
      } else {
        const i2 = leafOf(r[e2], t2);
        if (n in i2) return i2[n];
      }
    }
    return MISSING;
  }
  function mergeGet(e, n) {
    const t = mergeLookup(e, n);
    return t === MISSING ? void 0 : t;
  }
  function mergeHas(e, n) {
    const t = tableOf(e);
    if (t !== void 0) return t.has(n);
    const r = e.sources, i = e.kinds;
    for (let e2 = r.length - 1; e2 >= 0; e2--) if (sourceHas(r[e2], i[e2], n)) return true;
    return false;
  }
  function mergeDescriptor(e, n) {
    const t = tableOf(e);
    if (t !== void 0) return tableDescriptor(e, t, n);
    const r = e.sources, i = e.kinds;
    for (let t2 = r.length - 1; t2 >= 0; t2--) {
      if (!sourceHas(r[t2], i[t2], n)) continue;
      return sourceDescriptor(r[t2], i[t2], n, true) ?? accessorDescriptor(() => mergeGet(e, n));
    }
    return void 0;
  }
  function mergeKeysOf(e, n, t) {
    const r = [];
    collectKeys(e, t === void 0 ? void 0 : [t], n, r, null);
    return r;
  }
  function collectKeys(e, n, t, r, i) {
    const f = e.sources, u = e.kinds;
    for (let e2 = 0; e2 < f.length; e2++) {
      let o = f[e2], c = u[e2];
      let s;
      if (c === SOURCE_OMIT) {
        if (o.kind === SOURCE_MERGE) {
          if (n === void 0) n = [o];
          else n.push(o);
          collectKeys(o.source, n, t, r, i);
          n.pop();
          continue;
        }
        s = o;
        c = o.kind;
        o = o.source;
      }
      o = leafOf(o, c);
      const d = t ? ownEnumerableKeys(o) : leafKeys(o, c);
      for (let e3 = 0; e3 < d.length; e3++) {
        const t2 = d[e3];
        if (s !== void 0 && isHidden(s, t2)) continue;
        if (hiddenByAny(n, t2)) continue;
        addKey(r, i, t2, o);
      }
    }
  }
  function addKey(e, n, t, r) {
    const i = e.indexOf(t);
    if (i !== -1) {
      e.splice(i, 1);
      if (n !== null) n.splice(i, 1);
    }
    e.push(t);
    if (n !== null) n.push(r);
  }
  function mergeEnumerableKeys(e, n) {
    const t = mergeTable(e);
    if (t === void 0) return mergeKeysOf(e, true, n);
    const r = tableOwnKeys(e, t);
    if (n === void 0) return r;
    const i = [];
    for (let e2 = 0; e2 < r.length; e2++) if (!isHidden(n, r[e2])) i.push(r[e2]);
    return i;
  }
  var mergeTraps = {
    get(e, n, t) {
      if (typeof n === "symbol") {
        if (n === $PROXY) return t;
        if (n === $RECORD) return e;
        if (n === $TARGET) return void 0;
      }
      const r = e.table;
      let i;
      if (typeof r !== "object") {
        if (r + 1 < READS_FOR_TABLE) {
          e.table = r + 1;
          const t2 = e.sources, i2 = e.kinds;
          for (let r2 = t2.length - 1; r2 >= 0; r2--) {
            if (i2[r2] !== SOURCE_PLAIN) return mergeGet(e, n);
            const f2 = t2[r2][n];
            if (f2 !== void 0 || n in t2[r2]) return f2;
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
      const t = mergeReadTable(e);
      if (t !== void 0) return t.has(n);
      return mergeHas(e, n);
    },
    set: trueFn,
    deleteProperty: trueFn,
    getOwnPropertyDescriptor(e, n) {
      if (n === $PROXY || n === $TARGET || n === $RECORD) return void 0;
      const t = mergeReadTable(e);
      if (t !== void 0) return tableDescriptor(e, t, n);
      return mergeDescriptor(e, n);
    },
    ownKeys(e) {
      return mergeEnumerableKeys(e);
    }
  };
  function merge2(...e) {
    if (e.length === 1 && typeof e[0] !== "function") return e[0];
    const n = new Array(e.length);
    const t = new Array(e.length);
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
        t[r++] = SOURCE_MEMO;
        continue;
      }
      if ($PROXY in o2) {
        const e2 = recordOf(o2);
        if (e2 instanceof MergeView) {
          for (let i2 = 0; i2 < e2.sources.length; i2++) {
            n[r] = e2.sources[i2];
            t[r++] = e2.kinds[i2];
          }
        } else if (e2 !== void 0) {
          n[r] = e2;
          t[r++] = SOURCE_OMIT;
        } else {
          n[r] = o2;
          t[r++] = SOURCE_PROXY;
        }
        continue;
      }
      n[r] = o2;
      t[r++] = SOURCE_PLAIN;
    }
    if (r !== n.length) {
      n.length = r;
      t.length = r;
    }
    if (SUPPORTS_PROXY) {
      if (f === 1 && typeof i !== "function") return i;
      return new Proxy(new MergeView(n, t), mergeTraps);
    }
    const u = /* @__PURE__ */ Object.create(null);
    let o = false;
    let c = n.length - 1;
    for (let e2 = c; e2 >= 0; e2--) {
      const t2 = n[e2];
      if (!t2) {
        e2 === c && c--;
        continue;
      }
      const r2 = Object.getOwnPropertyNames(t2);
      for (let n2 = r2.length - 1; n2 >= 0; n2--) {
        const i2 = r2[n2];
        if (i2 === "__proto__" || i2 === "constructor") continue;
        if (!u[i2]) {
          o = o || e2 !== c;
          const n3 = Object.getOwnPropertyDescriptor(t2, i2);
          u[i2] = n3.get ? {
            enumerable: true,
            configurable: true,
            get: n3.get.bind(t2)
          } : n3;
        }
      }
    }
    if (!o) return n[c];
    const s = {};
    const d = Object.keys(u);
    for (let e2 = d.length - 1; e2 >= 0; e2--) {
      const n2 = d[e2], t2 = u[n2];
      if (t2.get) Object.defineProperty(s, n2, t2);
      else s[n2] = t2.value;
    }
    return s;
  }

  // node_modules/.pnpm/solid-js@2.0.0-rc.17/node_modules/solid-js/dist/solid.js
  var IS_DEV = false;
  var $DEVCOMP = Symbol(0);
  function children(fn) {
    const c = createMemo(fn, {
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
        const fn = typeof child === "function" && child.length > 0;
        return fn ? keyed ? untrack(() => child(c), IS_DEV) : untrack(() => child(() => {
          if (!shown) throw narrowedError("Show");
          return conditionValue();
        }), IS_DEV) : child;
      }
      return props.fallback;
    }, {
      sync: true
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
  var effect2 = (fn, effectFn, options3) => createRenderEffect2(fn, effectFn, options3 ? {
    sync: true,
    ...options3,
    transparent: !options3.scope
  } : transparentOptions);
  var memo = (fn) => createMemo2(() => fn(), syncOptions);
  var named = (options3, fallback) => options3;
  var INNER_OWNED = {};
  function createRenderer({
    createElement: createElement2,
    createTextNode: createTextNode2,
    createSentinel = () => createTextNode2(""),
    isTextNode,
    replaceText,
    insertNode: insertNode2,
    removeNode,
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
      const t = typeof value, multi = marker !== void 0;
      if (t === "string" || t === "number") {
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
          const item = value[i], t = typeof item;
          if (t === "string" || t === "number") value[i] = createTextNode2(item);
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
            if (!map || !map.has(a[aStart])) removeNode(parentNode, a[aStart]);
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
              let i = aStart, sequence = 1, t;
              while (++i < aEnd && i < bEnd) {
                if ((t = map.get(a[i])) == null || t !== index + sequence) break;
                sequence++;
              }
              if (sequence > index - bStart) {
                const node = a[aStart];
                while (bStart < index) insertNode2(parentNode, b[bStart++], node);
              } else replaceNode(parentNode, b[bStart++], a[aStart++]);
            } else aStart++;
          } else removeNode(parentNode, a[aStart++]);
        }
      }
    }
    function cleanChildren(parent, current, marker, replacement) {
      if (marker === void 0) {
        let removed;
        while (removed = getFirstChild(parent)) removeNode(parent, removed);
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
            else isParent && removeNode(parent, el);
          } else inserted = true;
        }
      } else if (replacement) insertNode2(parent, replacement, marker);
    }
    function appendNodes(parent, array, marker) {
      for (let i = 0, len = array.length; i < len; i++) insertNode2(parent, array[i], marker);
    }
    function replaceNode(parent, newNode, oldNode) {
      insertNode2(parent, newNode, oldNode);
      removeNode(parent, oldNode);
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
        if (getParentNode(node) === parent) removeNode(parent, node);
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
      const resolved = [];
      const resolvedKinds = [];
      for (let i = 0; i < sources.length; i++) pushEntry(resolved, resolvedKinds, sources[i], kinds !== void 0 ? kinds[i] : SOURCE_MEMO);
      for (let i = 0; i < resolved.length; i++) collectProps(out, resolved[i], resolvedKinds[i], resolved, resolvedKinds, i + 1);
      return out;
    }
    function pushEntry(resolved, kinds, s, kind) {
      if (kind !== SOURCE_MEMO) {
        resolved.push(s);
        kinds.push(kind);
        return;
      }
      s = resolveSource(s);
      if (s == null) return;
      const view = viewOf(s);
      if (view instanceof OmitView) {
        resolved.push(view);
        kinds.push(SOURCE_OMIT);
      } else if (view !== void 0) {
        const f = view.sources, k = view.kinds;
        for (let j = 0; j < f.length; j++) pushEntry(resolved, kinds, f[j], k[j]);
      } else {
        resolved.push(s);
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
    function ref2(fn, element) {
      const resolved = untrack(fn);
      runWithOwner(null, () => applyRef2(resolved, element));
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

  // node_modules/.pnpm/@uifw+solid-renderer@file+..+..+frameworks+solid-renderer/node_modules/@uifw/solid-renderer/src/defaults.ts
  var FONT_SIZE = 16;
  var TEXT_COLOR = "#FFFFFF";
  var BUTTON_FILL = "#282828";
  var BUTTON_PRESSED = "#565C68";
  var BUTTON_LABEL = "#7CFF6B";

  // node_modules/.pnpm/@uifw+solid-renderer@file+..+..+frameworks+solid-renderer/node_modules/@uifw/solid-renderer/src/tree.ts
  function asNumber(value, fallback) {
    return typeof value === "number" ? value : fallback;
  }
  function alignName(value) {
    if (value === "center" || value === "end") {
      return value;
    }
    return "start";
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
      const record = value;
      return {
        w: asNumber(record.w, 0),
        h: asNumber(record.h, 0)
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
      host2.setSize(id, next.w, next.h);
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
            w: asNumber(value, 0)
          });
          return;
        case "h":
          writeSize(node.id, {
            h: asNumber(value, 0)
          });
          return;
        case "family":
          host2.setFamily(node.id, value == null ? "" : String(value));
          return;
        case "justify":
          host2.setJustify(node.id, justifyName(value));
          return;
        case "wrap":
          host2.setWrap(node.id, value === true);
          return;
        case "fill":
          if (value === true) {
            host2.setFill(node.id);
          }
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
            id = host2.scroll();
            break;
          case "sized": {
            const width = asNumber(staticProps?.w, 0);
            const height = asNumber(staticProps?.h, 0);
            id = host2.sized(width, height);
            sizes.set(id, {
              w: width,
              h: height
            });
            break;
          }
          case "transform":
            id = host2.transform();
            break;
          case "decorated":
            id = host2.decorated(colorBytes(staticProps?.color ?? BUTTON_FILL), asNumber(staticProps?.radius, 0));
            break;
          case "image":
            id = host2.image(staticProps?.source == null ? "" : String(staticProps.source), asNumber(staticProps?.w, 0), asNumber(staticProps?.h, 0));
            break;
          default:
            throw new Error(`unknown tag ${tag}`);
        }
        const node = wrap(id);
        if (staticProps !== void 0) {
          for (const [name, value] of Object.entries(staticProps)) {
            if (name === "gap" || name === "edges" || name === "columns" || name === "rows" || name === "color" || name === "radius" || name === "alpha" || name === "source" || name === "w" || name === "h") {
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
      }
    };
  }

  // node_modules/.pnpm/@uifw+solid-renderer@file+..+..+frameworks+solid-renderer/node_modules/@uifw/solid-renderer/runtime/host.ts
  var host = globalThis.core;

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
    const resolved = children(() => props.children);
    const node = options.createTextNode("");
    createRenderEffect2(() => ({
      value: textContent(resolved()),
      fontSize: props.fontSize,
      color: props.color,
      maxLines: props.maxLines,
      ellipsis: props.ellipsis,
      softWrap: props.softWrap,
      overflow: props.overflow,
      textAlign: props.textAlign,
      weight: props.weight,
      direction: props.direction,
      lineHeight: props.lineHeight,
      family: props.family
    }), (next) => {
      options.replaceText(node, next.value);
      if (next.fontSize !== void 0) {
        options.setProperty(node, "fontSize", next.fontSize);
      }
      if (next.color !== void 0) {
        options.setProperty(node, "color", next.color);
      }
      if (next.maxLines !== void 0) {
        options.setProperty(node, "maxLines", next.maxLines);
      }
      if (next.ellipsis !== void 0) {
        options.setProperty(node, "ellipsis", next.ellipsis);
      }
      if (next.softWrap !== void 0) {
        options.setProperty(node, "softWrap", next.softWrap);
      }
      if (next.overflow !== void 0) {
        options.setProperty(node, "overflow", next.overflow);
      }
      if (next.textAlign !== void 0) {
        options.setProperty(node, "textAlign", next.textAlign);
      }
      if (next.weight !== void 0) {
        options.setProperty(node, "weight", next.weight);
      }
      if (next.direction !== void 0) {
        options.setProperty(node, "direction", next.direction);
      }
      if (next.lineHeight !== void 0) {
        options.setProperty(node, "lineHeight", next.lineHeight);
      }
      if (next.family !== void 0) {
        options.setProperty(node, "family", next.family);
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
  function onFrame(fn) {
    const id = nextTick;
    nextTick += 1;
    ticks.set(id, fn);
    return () => {
      ticks.delete(id);
    };
  }
  function frameListenerCount() {
    return ticks.size;
  }
  function runFrames(targetMs, intervalMs) {
    for (const [id, fn] of ticks) {
      try {
        fn(targetMs, intervalMs);
      } catch (error) {
        ticks.delete(id);
        throw error;
      }
    }
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
  function requestFrame() {
    host.requestFrame();
  }
  function boot(component) {
    const root = scene.createElement("column", {
      gap: 0,
      fill: true
    });
    host.onFrame((targetMs, intervalMs) => {
      flush();
      runFrames(targetMs, intervalMs);
    });
    render(component, root);
  }

  // node_modules/.pnpm/@uifw+solid-ui@file+..+..+frameworks+solid-ui/node_modules/@uifw/solid-ui/src/Dialog.tsx
  function Dialog(props) {
    var _el$ = createElement("stack", {
      fill: true
    });
    insert(_el$, createComponent2(Show, {
      get when() {
        return props.open;
      },
      get children() {
        return createComponent2(DialogLayer, {
          get onClose() {
            return props.onClose;
          },
          get children() {
            return [(() => {
              var _el$2 = createElement("pointer", {
                fill: true,
                onClick: () => props.onClose()
              }), _el$3 = createElement("decorated", {
                color: "#00000080",
                radius: 0,
                fill: true
              });
              insertNode(_el$2, _el$3);
              return _el$2;
            })(), (() => {
              var _el$4 = createElement("column", {
                gap: 8
              });
              insert(_el$4, () => props.children);
              return _el$4;
            })()];
          }
        });
      }
    }));
    return _el$;
  }
  function DialogLayer(props) {
    let back = 0;
    onCleanup(() => {
      host.popFocusScope();
      if (back !== 0) {
        host.offBack(back);
      }
    });
    var _el$5 = createElement("stack", {
      fill: true
    });
    ref(() => (node) => {
      host.pushFocusScope(node.id);
      back = host.onBack(() => {
        props.onClose();
        return true;
      });
    }, _el$5);
    insert(_el$5, () => props.children);
    return _el$5;
  }

  // node_modules/.pnpm/@uifw+solid-ui@file+..+..+frameworks+solid-ui/node_modules/@uifw/solid-ui/src/Field.tsx
  var FIELD_BORDER = "#8A8A8A";
  var FIELD_BORDER_FOCUS = "#7CFF6B";
  var HINT = "#8A8A8A";
  var CURSOR = "#7CFF6B";
  var LINE = 22;
  var EDIT_LEFT = 1;
  var EDIT_RIGHT = 2;
  var EDIT_BACKSPACE = 3;
  var EDIT_INSERT = 4;
  var EDIT_COMPOSING = 5;
  var EDIT_SELECT_ALL = 9;
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
  function utf16Len(value) {
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
  function reveal(id) {
    const field = host.rect(id);
    let parent = host.parent(id);
    while (parent !== 0) {
      let offset;
      try {
        offset = host.scrollOffset(parent);
      } catch {
        parent = host.parent(parent);
        continue;
      }
      const view = host.rect(parent);
      let y = offset.y;
      const paintedBottom = field.y - y + field.h;
      if (paintedBottom > view.y + view.h) {
        y += paintedBottom - (view.y + view.h);
      }
      if (field.y - y < view.y) {
        y -= view.y - (field.y - y);
      }
      if (y < 0) {
        y = 0;
      }
      if (y !== offset.y) {
        host.setScrollOffset(parent, offset.x, y);
      }
      parent = host.parent(parent);
    }
  }
  function Field(props) {
    const [text, setText] = createSignal2("");
    const [composing, setComposing] = createSignal2(null);
    const [focused, setFocused] = createSignal2(false);
    const [blink, setBlink] = createSignal2(true);
    const [caretX, setCaretX] = createSignal2(0);
    const [caretY, setCaretY] = createSignal2(0);
    const [caretH, setCaretH] = createSignal2(LINE);
    const [base, setBase] = createSignal2(0);
    const [extent, setExtent] = createSignal2(0);
    const [rects, setRects] = createSignal2([]);
    let node = 0;
    let view = 0;
    const hint = () => text().length === 0 && composing() == null;
    const shown = () => {
      if (hint()) {
        return props.placeholder ?? "";
      }
      return text();
    };
    function leaf() {
      return node === 0 ? 0 : findText(node);
    }
    function edit(op, extra = "") {
      const id = leaf();
      if (id === 0) {
        return;
      }
      const next = host.textEdit(id, op, extra);
      setText(next.text);
      setBase(next.start);
      setExtent(next.end);
      host.setText(id, next.text.length === 0 ? props.placeholder ?? "" : next.text);
    }
    function keyboardCode() {
      if (props.keyboard && props.keyboard !== "text") {
        return KEYBOARD[props.keyboard] ?? 0;
      }
      return props.multiline ? 1 : 0;
    }
    function actionCode() {
      if (props.action) {
        return ACTION[props.action] ?? 0;
      }
      return props.multiline ? 7 : 1;
    }
    function publish(show) {
      if (!show) {
        host.setImeState(null);
        return;
      }
      const full = text();
      const extra = composing() ?? "";
      const caret = extent();
      const anchor = base();
      const composingStart = extra.length === 0 ? -1 : Math.max(0, utf16Len(full) - utf16Len(extra));
      const composingEnd = extra.length === 0 ? -1 : utf16Len(full);
      const id = leaf();
      let x = caretX();
      let y = caretY();
      let h = caretH();
      if (id !== 0 && view !== 0) {
        const textBox = host.rect(id);
        const offset = host.scrollOffset(view);
        x = textBox.x + caretX() - offset.x;
        y = textBox.y + caretY() - offset.y;
      }
      host.setImeConfig(keyboardCode(), actionCode());
      host.setImeState(full, anchor, caret, composingStart, composingEnd, x, y, 1, h || LINE);
    }
    function follow() {
      if (view === 0) {
        return;
      }
      const id = leaf();
      if (id === 0) {
        return;
      }
      const box = host.rect(view);
      const textBox = host.rect(id);
      const offset = host.scrollOffset(view);
      let x = offset.x;
      let y = offset.y;
      const left = textBox.x + caretX() - x;
      const top = textBox.y + caretY() - y;
      const bottom = top + caretH();
      if (left + 1 > box.x + box.w) {
        x += left + 1 - (box.x + box.w);
      }
      if (left < box.x) {
        x -= box.x - left;
      }
      if (bottom > box.y + box.h) {
        y += bottom - (box.y + box.h);
      }
      if (top < box.y) {
        y -= box.y - top;
      }
      if (x < 0) {
        x = 0;
      }
      if (y < 0) {
        y = 0;
      }
      if (x !== offset.x || y !== offset.y) {
        host.setScrollOffset(view, x, y);
      }
    }
    onCleanup(onFrame((targetMs) => {
      if (!focused()) {
        return;
      }
      const id = leaf();
      if (id !== 0 && text().length > 0) {
        const hit = host.caretAtUnit(id, extent());
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
    createRenderEffect2(() => [extent(), text(), base()], ([unit, value, anchor]) => {
      const id = leaf();
      if (id === 0 || value.length === 0) {
        setCaretX(0);
        setCaretY(0);
        setCaretH(LINE);
        setRects([]);
        return;
      }
      const hit = host.caretAtUnit(id, unit);
      setCaretX(hit.x);
      setCaretY(hit.y);
      setCaretH(hit.h || LINE);
      const start = Math.min(anchor, unit);
      const end = Math.max(anchor, unit);
      setRects(start === end ? [] : host.selectionRects(id, start, end));
      follow();
    });
    function move(collapsed, extend, shift) {
      edit(shift ? extend : collapsed);
      publish(true);
    }
    var _el$ = createElement("pointer", {
      min: {
        w: 280,
        h: 48
      },
      focusable: true,
      onFocus: () => {
        setFocused(true);
        setBlink(true);
        publish(true);
        reveal(node);
        requestFrame();
      },
      onBlur: () => {
        setFocused(false);
        publish(false);
      },
      onPress: () => {
        const id = leaf();
        if (id === 0) {
          return;
        }
        const point = host.pointerPosition();
        const caretHit = host.caretAt(id, point.x, point.y);
        edit(EDIT_CARET, String(caretHit.unit));
        publish(true);
      },
      onDragBy: () => {
        const id = leaf();
        if (id === 0) {
          return;
        }
        const point = host.pointerPosition();
        const caretHit = host.caretAt(id, point.x, point.y);
        edit(EDIT_EXTEND_CARET, String(caretHit.unit));
        publish(true);
      },
      onKey: (key) => {
        const shift = key.startsWith("Shift+");
        const name = shift ? key.slice("Shift+".length) : key;
        if (name === "Left") {
          move(EDIT_LEFT, EDIT_EXTEND_LEFT, shift);
          return;
        }
        if (name === "Right") {
          move(EDIT_RIGHT, EDIT_EXTEND_RIGHT, shift);
          return;
        }
        if (name === "Up") {
          if (props.multiline) {
            move(EDIT_UP, EDIT_EXTEND_UP, shift);
          } else {
            host.focusStep("up");
          }
          return;
        }
        if (name === "Down") {
          if (props.multiline) {
            move(EDIT_DOWN, EDIT_EXTEND_DOWN, shift);
          } else {
            host.focusStep("down");
          }
          return;
        }
        if (name === "Home") {
          move(EDIT_LINE_START, EDIT_EXTEND_LINE_START, shift);
          return;
        }
        if (name === "End") {
          move(EDIT_LINE_END, EDIT_EXTEND_LINE_END, shift);
          return;
        }
        if (name === "WordLeft") {
          move(EDIT_WORD_LEFT, EDIT_EXTEND_WORD_LEFT, shift);
          return;
        }
        if (name === "WordRight") {
          move(EDIT_WORD_RIGHT, EDIT_EXTEND_WORD_RIGHT, shift);
          return;
        }
        if (name === "DocumentStart") {
          move(EDIT_DOCUMENT_START, EDIT_EXTEND_DOCUMENT_START, shift);
          return;
        }
        if (name === "DocumentEnd") {
          move(EDIT_DOCUMENT_END, EDIT_EXTEND_DOCUMENT_END, shift);
          return;
        }
        if (name === "SelectAll") {
          edit(EDIT_SELECT_ALL);
          publish(true);
          return;
        }
        if (name === "Copy") {
          edit(10);
          return;
        }
        if (name === "Cut") {
          edit(11);
          publish(true);
          return;
        }
        if (name === "Backspace") {
          edit(EDIT_BACKSPACE);
          publish(true);
          return;
        }
        if (name === "Delete") {
          edit(EDIT_FORWARD_DELETE);
          publish(true);
          return;
        }
        if (name === "DeleteWord") {
          edit(EDIT_DELETE_WORD);
          publish(true);
          return;
        }
        if (name === "DeleteWordForward") {
          edit(EDIT_DELETE_WORD_FORWARD);
          publish(true);
          return;
        }
        if (name === "DeleteToLineStart") {
          edit(EDIT_DELETE_TO_LINE_START);
          publish(true);
          return;
        }
        if (name === "DeleteToLineEnd") {
          edit(EDIT_DELETE_TO_LINE_END);
          publish(true);
          return;
        }
        if (name === "Enter") {
          if (props.multiline && props.action !== "done" && props.action !== "search" && props.action !== "go" && props.action !== "send" && props.action !== "next") {
            edit(EDIT_NEWLINE);
            publish(true);
          } else {
            props.onInput?.(text());
          }
        }
      },
      onText: (value) => {
        setComposing(null);
        edit(EDIT_INSERT, value);
        publish(true);
      },
      onComposing: (value) => {
        setComposing(value);
        edit(EDIT_COMPOSING, value ?? "");
        publish(true);
      }
    }), _el$2 = createElement("padding", {
      edges: 8
    }), _el$3 = createElement("decorated", {
      radius: 6,
      fill: true,
      min: {
        w: 280,
        h: 48
      }
    }), _el$4 = createElement("padding", {
      edges: 1
    }), _el$5 = createElement("decorated", {
      color: "#1A1A1A",
      radius: 5,
      fill: true
    }), _el$6 = createElement("padding", {
      edges: 10
    }), _el$7 = createElement("scroll"), _el$8 = createElement("padding", {
      right: 2
    }), _el$9 = createElement("stack"), _el$0 = createElement("transform"), _el$1 = createElement("decorated"), _el$10 = createElement("sized", {
      w: 2
    });
    insertNode(_el$, _el$2);
    ref(() => (handle) => {
      node = handle.id;
      host.setDragClaim(handle.id, "horizontal");
    }, _el$);
    insertNode(_el$2, _el$3);
    insertNode(_el$3, _el$4);
    insertNode(_el$4, _el$5);
    insertNode(_el$5, _el$6);
    insertNode(_el$6, _el$7);
    insertNode(_el$7, _el$8);
    ref(() => (handle) => {
      view = handle.id;
    }, _el$7);
    insertNode(_el$8, _el$9);
    insertNode(_el$9, _el$0);
    insert(_el$9, createComponent2(Text, {
      fontSize: FONT_SIZE,
      get color() {
        return hint() ? HINT : TEXT_COLOR;
      },
      get softWrap() {
        return !!props.multiline;
      },
      get overflow() {
        return props.multiline ? "clip" : "visible";
      },
      get maxLines() {
        return memo2(() => !!props.multiline)() ? props.maxLines : void 0;
      },
      get children() {
        return shown();
      }
    }), _el$0);
    insert(_el$9, () => rects().map((rect) => (() => {
      var _el$11 = createElement("transform"), _el$12 = createElement("decorated", {
        color: "#7CFF6B66"
      }), _el$13 = createElement("sized");
      insertNode(_el$11, _el$12);
      insertNode(_el$12, _el$13);
      effect3(() => ({
        e: rect.x,
        t: rect.y,
        a: rect.w,
        o: rect.h
      }), ({
        e,
        t,
        a,
        o
      }, _p$) => {
        e !== _p$?.e && setProp(_el$11, "x", e, _p$?.e);
        t !== _p$?.t && setProp(_el$11, "y", t, _p$?.t);
        a !== _p$?.a && setProp(_el$13, "w", a, _p$?.a);
        o !== _p$?.o && setProp(_el$13, "h", o, _p$?.o);
      });
      return _el$11;
    })()), _el$0);
    insertNode(_el$0, _el$1);
    insertNode(_el$1, _el$10);
    effect3(() => ({
      e: focused() ? FIELD_BORDER_FOCUS : FIELD_BORDER,
      t: caretX(),
      a: caretY(),
      o: focused() && blink() && base() === extent() ? CURSOR : "#00000000",
      i: caretH()
    }), ({
      e,
      t,
      a,
      o,
      i
    }, _p$) => {
      e !== _p$?.e && setProp(_el$3, "color", e, _p$?.e);
      t !== _p$?.t && setProp(_el$0, "x", t, _p$?.t);
      a !== _p$?.a && setProp(_el$0, "y", a, _p$?.a);
      o !== _p$?.o && setProp(_el$1, "color", o, _p$?.o);
      i !== _p$?.i && setProp(_el$10, "h", i, _p$?.i);
    });
    return _el$;
  }

  // node_modules/.pnpm/@uifw+solid-ui@file+..+..+frameworks+solid-ui/node_modules/@uifw/solid-ui/src/ScrollArea.tsx
  function ScrollArea(props) {
    const [off, setOff] = createSignal2({
      x: 0,
      y: 0
    });
    let viewport = 0;
    let content = 0;
    function clamp(x, y) {
      if (viewport === 0 || content === 0) {
        return {
          x: 0,
          y: 0
        };
      }
      const view = host.rect(viewport);
      const body = host.rect(content);
      const maxX = Math.max(0, body.w - view.w);
      const maxY = Math.max(0, body.h - view.h);
      return {
        x: Math.min(maxX, Math.max(0, x)),
        y: Math.min(maxY, Math.max(0, y))
      };
    }
    createRenderEffect2(off, (next) => {
      if (viewport !== 0) {
        host.setScrollOffset(viewport, next.x, next.y);
        props.onOffset?.(next, viewport);
      }
    });
    var _el$ = createElement("scroll", {
      onScroll: (x, y) => {
        setOff(clamp(x, y));
      },
      onDragBy: (dx, dy) => {
        const live = viewport === 0 ? off() : host.scrollOffset(viewport);
        setOff(clamp(live.x - dx, live.y - dy));
      },
      onWheel: (dx, dy) => {
        const live = viewport === 0 ? off() : host.scrollOffset(viewport);
        setOff(clamp(live.x + dx, live.y + dy));
      }
    }), _el$2 = createElement("column");
    insertNode(_el$, _el$2);
    ref(() => (node) => {
      viewport = node.id;
      props.viewportRef?.(node.id);
    }, _el$);
    ref(() => (node) => {
      content = node.id;
    }, _el$2);
    insert(_el$2, () => props.children);
    return _el$;
  }

  // node_modules/.pnpm/@uifw+solid-ui@file+..+..+frameworks+solid-ui/node_modules/@uifw/solid-ui/src/VirtualList.tsx
  function VirtualList(props) {
    const [range, setRange] = createSignal2({
      start: 0,
      end: 1,
      top: 0,
      bottom: 0
    });
    let viewport = 0;
    function heightAt(index) {
      const height = props.itemHeight;
      const value = typeof height === "number" ? height : height(index);
      return value > 0 ? value : 0;
    }
    function place(offsetY, viewHeight) {
      const count = props.count;
      if (count <= 0) {
        setRange({
          start: 0,
          end: 0,
          top: 0,
          bottom: 0
        });
        return;
      }
      const prefix = [0];
      for (let index = 0; index < count; index += 1) {
        prefix.push((prefix[index] ?? 0) + heightAt(index));
      }
      const overscan = props.overscan ?? 4;
      let start = 0;
      while (start + 1 < count && (prefix[start + 1] ?? 0) <= offsetY) {
        start += 1;
      }
      start = Math.max(0, start - overscan);
      let end = start;
      const limit = offsetY + Math.max(viewHeight, 0);
      while (end < count && (prefix[end] ?? 0) < limit) {
        end += 1;
      }
      end = Math.min(count, end + overscan);
      const top = prefix[start] ?? 0;
      const bottom = (prefix[count] ?? 0) - (prefix[end] ?? 0);
      const current = range();
      if (current.start !== start || current.end !== end || current.top !== top || current.bottom !== bottom) {
        setRange({
          start,
          end,
          top,
          bottom
        });
      }
    }
    const stop = onFrame(() => {
      if (viewport === 0) {
        return;
      }
      const view = host.rect(viewport);
      place(host.scrollOffset(viewport).y, view.h);
      stop();
    });
    onCleanup(stop);
    requestFrame();
    const indexes = () => {
      const windowed = range();
      const out = [];
      for (let index = windowed.start; index < windowed.end; index += 1) {
        out.push(index);
      }
      return out;
    };
    return createComponent2(ScrollArea, {
      viewportRef: (id) => {
        viewport = id;
      },
      onOffset: (offset, id) => {
        place(offset.y, host.rect(id).h);
      },
      get children() {
        return [createComponent2(Show, {
          get when() {
            return range().top > 0;
          },
          get children() {
            var _el$ = createElement("sized", {
              w: 1
            });
            effect3(() => range().top, (_v$, _$p) => {
              setProp(_el$, "h", _v$, _$p);
            });
            return _el$;
          }
        }), createComponent2(For, {
          get each() {
            return indexes();
          },
          children: (index) => props.children(index)
        }), createComponent2(Show, {
          get when() {
            return range().bottom > 0;
          },
          get children() {
            var _el$2 = createElement("sized", {
              w: 1
            });
            effect3(() => range().bottom, (_v$, _$p) => {
              setProp(_el$2, "h", _v$, _$p);
            });
            return _el$2;
          }
        })];
      }
    });
  }

  // node_modules/.pnpm/@uifw+solid-ui@file+..+..+frameworks+solid-ui/node_modules/@uifw/solid-ui/src/index.tsx
  function Button(props) {
    const [pressed, setPressed] = createSignal2(false);
    var _el$ = createElement("pointer", {
      onPress: () => {
        if (!props.disabled) {
          setPressed(true);
        }
      },
      onRelease: () => setPressed(false),
      onClick: () => {
        if (!props.disabled) {
          props.onClick();
        }
      }
    }), _el$2 = createElement("padding", {
      edges: 8
    }), _el$3 = createElement("decorated", {
      radius: 6
    });
    insertNode(_el$, _el$2);
    ref(() => (node) => host.setMinSize(node.id, 44, 44), _el$);
    insertNode(_el$2, _el$3);
    insert(_el$3, createComponent2(Text, {
      fontSize: FONT_SIZE,
      color: BUTTON_LABEL,
      get children() {
        return props.label;
      }
    }));
    effect3(() => pressed() ? BUTTON_PRESSED : BUTTON_FILL, (_v$, _$p) => {
      setProp(_el$3, "color", _v$, _$p);
    });
    return _el$;
  }

  // counter.tsx
  function SchedulePage() {
    const [label, setLabel] = createSignal2("\u8FD8\u662F\u65E7\u7684");
    var _el$ = createElement("column", {
      gap: 8
    });
    insert(_el$, createComponent2(Text, {
      fontSize: 16,
      color: "#FFFFFF",
      get children() {
        return label();
      }
    }), null);
    insert(_el$, createComponent2(Button, {
      label: "\u5FAE\u4EFB\u52A1\u6539\u5B57",
      onClick: () => {
        queueMicrotask(() => setLabel("\u5DF2\u7ECF\u6539\u4E86"));
      }
    }), null);
    return _el$;
  }
  function NavPage() {
    const [depth, setDepth] = createSignal2(0);
    const [dialog, setDialog] = createSignal2(false);
    const [listeners, setListeners] = createSignal2(frameListenerCount());
    onCleanup(onFrame(() => setListeners(frameListenerCount())));
    const back = host.onBack(() => {
      if (depth() === 0) {
        return false;
      }
      setDepth(depth() - 1);
      return true;
    });
    onCleanup(() => host.offBack(back));
    var _el$2 = createElement("column", {
      gap: 8
    });
    insert(_el$2, createComponent2(Text, {
      fontSize: 16,
      color: "#FFFFFF",
      get children() {
        return `\u7B2C ${depth() + 1} \u9875`;
      }
    }), null);
    insert(_el$2, createComponent2(Text, {
      fontSize: 16,
      color: "#FFFFFF",
      get children() {
        return `\u76D1\u542C ${listeners()}`;
      }
    }), null);
    insert(_el$2, createComponent2(Button, {
      label: "\u63A8\u8FDB",
      onClick: () => setDepth(depth() + 1)
    }), null);
    insert(_el$2, createComponent2(Button, {
      label: "\u5F39\u51FA",
      onClick: () => setDepth(Math.max(0, depth() - 1))
    }), null);
    insert(_el$2, createComponent2(Button, {
      label: "\u5BF9\u8BDD\u6846",
      onClick: () => setDialog(true)
    }), null);
    insert(_el$2, createComponent2(Dialog, {
      get open() {
        return dialog();
      },
      onClose: () => setDialog(false),
      get children() {
        return createComponent2(Text, {
          fontSize: 16,
          color: "#FFFFFF",
          children: "\u8FD4\u56DE\u952E\u5173\u95ED"
        });
      }
    }), null);
    return _el$2;
  }
  function ListPage() {
    const [scale, setScale] = createSignal2(1);
    const [missing, setMissing] = createSignal2(false);
    const [name, setName] = createSignal2("");
    var _el$3 = createElement("column", {
      gap: 8,
      fill: true
    }), _el$4 = createElement("row", {
      gap: 8
    });
    insertNode(_el$3, _el$4);
    insert(_el$4, createComponent2(Button, {
      get label() {
        return scale() === 1 ? "\u5B57\u53F7 2.0" : "\u5B57\u53F7 1.0";
      },
      onClick: () => {
        const next = scale() === 1 ? 2 : 1;
        setScale(next);
        host.setTextScale(next);
      }
    }), null);
    insert(_el$4, createComponent2(Button, {
      get label() {
        return missing() ? "\u53BB\u6389\u7F3A\u56FE" : "\u7F3A\u56FE";
      },
      onClick: () => setMissing(!missing())
    }), null);
    insert(_el$3, createComponent2(Show, {
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
    }), null);
    insert(_el$3, createComponent2(Field, {
      placeholder: "\u59D3\u540D",
      onInput: setName
    }), null);
    insert(_el$3, createComponent2(Text, {
      fontSize: 16,
      color: "#FFFFFF",
      get children() {
        return name() || "\u672A\u586B\u5199";
      }
    }), null);
    insert(_el$3, createComponent2(VirtualList, {
      count: 200,
      itemHeight: 28,
      children: (index) => createComponent2(Text, {
        fontSize: 16,
        color: "#FFFFFF",
        children: `Item ${index}`
      })
    }), null);
    return _el$3;
  }
  function App() {
    const [page, setPage] = createSignal2("schedule");
    var _el$6 = createElement("column", {
      gap: 8,
      fill: true
    }), _el$7 = createElement("row", {
      gap: 8
    });
    insertNode(_el$6, _el$7);
    insert(_el$7, createComponent2(Button, {
      label: "\u8C03\u5EA6",
      onClick: () => setPage("schedule")
    }), null);
    insert(_el$7, createComponent2(Button, {
      label: "\u5BFC\u822A",
      onClick: () => setPage("nav")
    }), null);
    insert(_el$7, createComponent2(Button, {
      label: "\u5217\u8868",
      onClick: () => setPage("list")
    }), null);
    insert(_el$6, createComponent2(Show, {
      get when() {
        return page() === "schedule";
      },
      get children() {
        return createComponent2(SchedulePage, {});
      }
    }), null);
    insert(_el$6, createComponent2(Show, {
      get when() {
        return page() === "nav";
      },
      get children() {
        return createComponent2(NavPage, {});
      }
    }), null);
    insert(_el$6, createComponent2(Show, {
      get when() {
        return page() === "list";
      },
      get children() {
        return createComponent2(ListPage, {});
      }
    }), null);
    return _el$6;
  }
  boot(App);
})();
//# sourceMappingURL=counter.js.map

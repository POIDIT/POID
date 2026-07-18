var m = ((t) =>
  typeof require < "u"
    ? require
    : typeof Proxy < "u"
      ? new Proxy(t, { get: (n, l) => (typeof require < "u" ? require : n)[l] })
      : t)(function (t) {
  if (typeof require < "u") return require.apply(this, arguments);
  throw Error('Dynamic require of "' + t + '" is not supported');
});
var ue = Object.create,
  U = Object.defineProperty,
  oe = Object.getOwnPropertyDescriptor,
  A = Object.getOwnPropertyNames,
  ae = Object.getPrototypeOf,
  ie = Object.prototype.hasOwnProperty,
  z = (t, n) => () => (n || (0, t[A(t)[0]])((n = { exports: {} }).exports, n), n.exports),
  fe = (t, n, l, _) => {
    if ((n && typeof n == "object") || typeof n == "function")
      for (const d of A(n))
        !ie.call(t, d) &&
          d !== l &&
          U(t, d, { get: () => n[d], enumerable: !(_ = oe(n, d)) || _.enumerable });
    return t;
  },
  le = (t, n, l) => (
    (l = t != null ? ue(ae(t)) : {}),
    fe(n || !t || !t.__esModule ? U(l, "default", { value: t, enumerable: !0 }) : l, t)
  ),
  ce = z({
    "cjs/react.production.min.js"(t) {
      var n = Symbol.for("react.element"),
        l = Symbol.for("react.portal"),
        _ = Symbol.for("react.fragment"),
        d = Symbol.for("react.strict_mode"),
        g = Symbol.for("react.profiler"),
        Y = Symbol.for("react.provider"),
        G = Symbol.for("react.context"),
        K = Symbol.for("react.forward_ref"),
        Q = Symbol.for("react.suspense"),
        X = Symbol.for("react.memo"),
        x = Symbol.for("react.lazy"),
        k = Symbol.iterator;
      function Z(e) {
        return e === null || typeof e != "object"
          ? null
          : ((e = (k && e[k]) || e["@@iterator"]), typeof e == "function" ? e : null);
      }
      var $ = {
          isMounted: () => !1,
          enqueueForceUpdate: () => {},
          enqueueReplaceState: () => {},
          enqueueSetState: () => {},
        },
        I = Object.assign,
        q = {};
      function h(e, r, o) {
        (this.props = e), (this.context = r), (this.refs = q), (this.updater = o || $);
      }
      (h.prototype.isReactComponent = {}),
        (h.prototype.setState = function (e, r) {
          if (typeof e != "object" && typeof e != "function" && e != null)
            throw Error(
              "setState(...): takes an object of state variables to update or a function which returns an object of state variables.",
            );
          this.updater.enqueueSetState(this, e, r, "setState");
        }),
        (h.prototype.forceUpdate = function (e) {
          this.updater.enqueueForceUpdate(this, e, "forceUpdate");
        });
      function D() {}
      D.prototype = h.prototype;
      function O(e, r, o) {
        (this.props = e), (this.context = r), (this.refs = q), (this.updater = o || $);
      }
      var P = (O.prototype = new D());
      (P.constructor = O), I(P, h.prototype), (P.isPureReactComponent = !0);
      var T = Array.isArray,
        V = Object.prototype.hasOwnProperty,
        b = { current: null },
        M = { key: !0, ref: !0, __self: !0, __source: !0 };
      function N(e, r, o) {
        var i,
          a = {},
          c = null,
          y = null;
        if (r != null)
          for (i in (r.ref !== void 0 && (y = r.ref), r.key !== void 0 && (c = "" + r.key), r))
            V.call(r, i) && !Object.hasOwn(M, i) && (a[i] = r[i]);
        var s = arguments.length - 2;
        if (s === 1) a.children = o;
        else if (1 < s) {
          for (var f = Array(s), v = 0; v < s; v++) f[v] = arguments[v + 2];
          a.children = f;
        }
        if (e && e.defaultProps)
          for (i in ((s = e.defaultProps), s)) a[i] === void 0 && (a[i] = s[i]);
        return { $$typeof: n, type: e, key: c, ref: y, props: a, _owner: b.current };
      }
      function ee(e, r) {
        return { $$typeof: n, type: e.type, key: r, ref: e.ref, props: e.props, _owner: e._owner };
      }
      function w(e) {
        return typeof e == "object" && e !== null && e.$$typeof === n;
      }
      function te(e) {
        var r = { "=": "=0", ":": "=2" };
        return "$" + e.replace(/[=:]/g, (o) => r[o]);
      }
      var L = /\/+/g;
      function C(e, r) {
        return typeof e == "object" && e !== null && e.key != null
          ? te("" + e.key)
          : r.toString(36);
      }
      function E(e, r, o, i, a) {
        var c = typeof e;
        (c === "undefined" || c === "boolean") && (e = null);
        var y = !1;
        if (e === null) y = !0;
        else
          switch (c) {
            case "string":
            case "number":
              y = !0;
              break;
            case "object":
              switch (e.$$typeof) {
                case n:
                case l:
                  y = !0;
              }
          }
        if (y)
          return (
            (y = e),
            (a = a(y)),
            (e = i === "" ? "." + C(y, 0) : i),
            T(a)
              ? ((o = ""), e != null && (o = e.replace(L, "$&/") + "/"), E(a, r, o, "", (v) => v))
              : a != null &&
                (w(a) &&
                  (a = ee(
                    a,
                    o +
                      (!a.key || (y && y.key === a.key)
                        ? ""
                        : ("" + a.key).replace(L, "$&/") + "/") +
                      e,
                  )),
                r.push(a)),
            1
          );
        if (((y = 0), (i = i === "" ? "." : i + ":"), T(e)))
          for (var s = 0; s < e.length; s++) {
            c = e[s];
            var f = i + C(c, s);
            y += E(c, r, o, f, a);
          }
        else if (((f = Z(e)), typeof f == "function"))
          for (e = f.call(e), s = 0; !(c = e.next()).done; )
            (c = c.value), (f = i + C(c, s++)), (y += E(c, r, o, f, a));
        else if (c === "object")
          throw (
            ((r = String(e)),
            Error(
              "Objects are not valid as a React child (found: " +
                (r === "[object Object]"
                  ? "object with keys {" + Object.keys(e).join(", ") + "}"
                  : r) +
                "). If you meant to render a collection of children, use an array instead.",
            ))
          );
        return y;
      }
      function S(e, r, o) {
        if (e == null) return e;
        var i = [],
          a = 0;
        return E(e, i, "", "", (c) => r.call(o, c, a++)), i;
      }
      function re(e) {
        if (e._status === -1) {
          var r = e._result;
          (r = r()),
            r.then(
              (o) => {
                (e._status === 0 || e._status === -1) && ((e._status = 1), (e._result = o));
              },
              (o) => {
                (e._status === 0 || e._status === -1) && ((e._status = 2), (e._result = o));
              },
            ),
            e._status === -1 && ((e._status = 0), (e._result = r));
        }
        if (e._status === 1) return e._result.default;
        throw e._result;
      }
      var p = { current: null },
        R = { transition: null },
        ne = { ReactCurrentDispatcher: p, ReactCurrentBatchConfig: R, ReactCurrentOwner: b };
      function F() {
        throw Error("act(...) is not supported in production builds of React.");
      }
      (t.Children = {
        map: S,
        forEach: (e, r, o) => {
          S(
            e,
            function () {
              r.apply(this, arguments);
            },
            o,
          );
        },
        count: (e) => {
          var r = 0;
          return (
            S(e, () => {
              r++;
            }),
            r
          );
        },
        toArray: (e) => S(e, (r) => r) || [],
        only: (e) => {
          if (!w(e))
            throw Error("React.Children.only expected to receive a single React element child.");
          return e;
        },
      }),
        (t.Component = h),
        (t.Fragment = _),
        (t.Profiler = g),
        (t.PureComponent = O),
        (t.StrictMode = d),
        (t.Suspense = Q),
        (t.__SECRET_INTERNALS_DO_NOT_USE_OR_YOU_WILL_BE_FIRED = ne),
        (t.act = F),
        (t.cloneElement = function (e, r, o) {
          if (e == null)
            throw Error(
              "React.cloneElement(...): The argument must be a React element, but you passed " +
                e +
                ".",
            );
          var i = I({}, e.props),
            a = e.key,
            c = e.ref,
            y = e._owner;
          if (r != null) {
            if (
              (r.ref !== void 0 && ((c = r.ref), (y = b.current)),
              r.key !== void 0 && (a = "" + r.key),
              e.type && e.type.defaultProps)
            )
              var s = e.type.defaultProps;
            for (f in r)
              V.call(r, f) &&
                !Object.hasOwn(M, f) &&
                (i[f] = r[f] === void 0 && s !== void 0 ? s[f] : r[f]);
          }
          var f = arguments.length - 2;
          if (f === 1) i.children = o;
          else if (1 < f) {
            s = Array(f);
            for (var v = 0; v < f; v++) s[v] = arguments[v + 2];
            i.children = s;
          }
          return { $$typeof: n, type: e.type, key: a, ref: c, props: i, _owner: y };
        }),
        (t.createContext = (e) => (
          (e = {
            $$typeof: G,
            _currentValue: e,
            _currentValue2: e,
            _threadCount: 0,
            Provider: null,
            Consumer: null,
            _defaultValue: null,
            _globalName: null,
          }),
          (e.Provider = { $$typeof: Y, _context: e }),
          (e.Consumer = e)
        )),
        (t.createElement = N),
        (t.createFactory = (e) => {
          var r = N.bind(null, e);
          return (r.type = e), r;
        }),
        (t.createRef = () => ({ current: null })),
        (t.forwardRef = (e) => ({ $$typeof: K, render: e })),
        (t.isValidElement = w),
        (t.lazy = (e) => ({ $$typeof: x, _payload: { _status: -1, _result: e }, _init: re })),
        (t.memo = (e, r) => ({ $$typeof: X, type: e, compare: r === void 0 ? null : r })),
        (t.startTransition = (e) => {
          var r = R.transition;
          R.transition = {};
          try {
            e();
          } finally {
            R.transition = r;
          }
        }),
        (t.unstable_act = F),
        (t.useCallback = (e, r) => p.current.useCallback(e, r)),
        (t.useContext = (e) => p.current.useContext(e)),
        (t.useDebugValue = () => {}),
        (t.useDeferredValue = (e) => p.current.useDeferredValue(e)),
        (t.useEffect = (e, r) => p.current.useEffect(e, r)),
        (t.useId = () => p.current.useId()),
        (t.useImperativeHandle = (e, r, o) => p.current.useImperativeHandle(e, r, o)),
        (t.useInsertionEffect = (e, r) => p.current.useInsertionEffect(e, r)),
        (t.useLayoutEffect = (e, r) => p.current.useLayoutEffect(e, r)),
        (t.useMemo = (e, r) => p.current.useMemo(e, r)),
        (t.useReducer = (e, r, o) => p.current.useReducer(e, r, o)),
        (t.useRef = (e) => p.current.useRef(e)),
        (t.useState = (e) => p.current.useState(e)),
        (t.useSyncExternalStore = (e, r, o) => p.current.useSyncExternalStore(e, r, o)),
        (t.useTransition = () => p.current.useTransition()),
        (t.version = "18.3.1");
    },
  }),
  se = z({
    "index.js"(t, n) {
      n.exports = ce();
    },
  }),
  u = le(se()),
  Pe = u.default,
  be = u.default.Children,
  we = u.default.Component,
  Ce = u.default.Fragment,
  je = u.default.Profiler,
  ge = u.default.PureComponent,
  ke = u.default.StrictMode,
  $e = u.default.Suspense,
  Ie = u.default.cloneElement,
  qe = u.default.createContext,
  De = u.default.createElement,
  Te = u.default.createRef,
  Ve = u.default.forwardRef,
  Me = u.default.isValidElement,
  Ne = u.default.lazy,
  Le = u.default.memo,
  Fe = u.default.startTransition,
  Ue = u.default.useCallback,
  Ae = u.default.useContext,
  ze = u.default.useDebugValue,
  He = u.default.useDeferredValue,
  Be = u.default.useEffect,
  Je = u.default.useId,
  We = u.default.useImperativeHandle,
  Ye = u.default.useInsertionEffect,
  Ge = u.default.useLayoutEffect,
  Ke = u.default.useMemo,
  Qe = u.default.useReducer,
  Xe = u.default.useRef,
  H = u.default.useState,
  xe = u.default.useSyncExternalStore,
  Ze = u.default.useTransition,
  et = u.default.version;
var ye = Object.create,
  B = Object.defineProperty,
  de = Object.getOwnPropertyDescriptor,
  J = Object.getOwnPropertyNames,
  pe = Object.getPrototypeOf,
  ve = Object.prototype.hasOwnProperty,
  _e = ((t) =>
    typeof m < "u"
      ? m
      : typeof Proxy < "u"
        ? new Proxy(t, { get: (n, l) => (typeof m < "u" ? m : n)[l] })
        : t)(function (t) {
    if (typeof m < "u") return m.apply(this, arguments);
    throw Error('Dynamic require of "' + t + '" is not supported');
  }),
  me = (t, n) => () => (n || (0, t[J(t)[0]])((n = { exports: {} }).exports, n), n.exports),
  he = (t, n, l, _) => {
    if ((n && typeof n == "object") || typeof n == "function")
      for (const d of J(n))
        !ve.call(t, d) &&
          d !== l &&
          B(t, d, { get: () => n[d], enumerable: !(_ = de(n, d)) || _.enumerable });
    return t;
  },
  Ee = (t, n, l) => (
    (l = t != null ? ye(pe(t)) : {}),
    he(n || !t || !t.__esModule ? B(l, "default", { value: t, enumerable: !0 }) : l, t)
  ),
  Se = me({
    "client.js"(t) {
      var n = _e("react-dom");
      (t.createRoot = n.createRoot), (t.hydrateRoot = n.hydrateRoot);
      var l;
    },
  }),
  j = Ee(Se()),
  rt = j.default,
  W = j.default.createRoot,
  nt = j.default.hydrateRoot;
var Re = document.createElement("div");
W(Re).render(H);

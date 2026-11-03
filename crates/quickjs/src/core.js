(function (global) {
  "use strict";
  var core = global.core;
  var abi = global.__abi;
  var OP = abi.MutationOp;
  var FIELD = abi.StyleField;
  var CREATE = abi.CreateKind;
  var QUERY = abi.QueryKind;
  var EFFECT = abi.EffectKind;

  function handle(value) {
    var number = +value;
    if (!(number >= 0 && number < 9007199254740992) || number !== Math.floor(number)) {
      throw new TypeError("bad handle");
    }
    return number;
  }

  function colorOf(value) {
    if (!value || value.length !== 4) {
      throw new Error("color must be [r,g,b,a]");
    }
    return [value[0] | 0, value[1] | 0, value[2] | 0, value[3] | 0];
  }

  function mutate(op, id, parent, before, field, n, n2, n3, n4, color, text) {
    return core._mutate(op, id || 0, parent || 0, before || 0, field || 0, n || 0, n2 || 0, n3 || 0, n4 || 0, color || [0, 0, 0, 0], text || "");
  }

  function must(result, message) {
    if (!result || !result.ok) {
      throw new Error(message);
    }
    return result.id || 0;
  }

  function write(op, id, parent, before, field, n, n2, n3, n4, color, text, message) {
    must(mutate(op, id, parent, before, field, n, n2, n3, n4, color, text), message);
  }

  function style(node, field, n, n2, n3, n4, color, text, message) {
    write(OP.SetStyle, handle(node), 0, 0, field, n, n2, n3, n4, color, text, message);
  }

  function create(kind, n, n2, color, text, message) {
    return must(mutate(OP.Create, 0, 0, 0, kind, n, n2, 0, 0, color, text), message);
  }

  function named(value, table, message) {
    if (!Object.prototype.hasOwnProperty.call(table, value)) {
      throw new Error(message);
    }
    return table[value];
  }

  var ALIGN = { start: 0, center: 1, end: 2 };
  var CROSS = { start: 0, center: 1, end: 2, stretch: 3, baseline: 4 };
  var JUSTIFY = { start: 0, center: 1, end: 2, "space-between": 3, "space-around": 4, "space-evenly": 5 };
  var TEXT_ALIGN = { left: 0, center: 1, right: 2 };
  var IMAGE_MODE = { fill: 0, fit: 1, crop: 2 };

  function axisTag(value) {
    if (value === "auto") {
      return [0, 0];
    }
    if (typeof value === "string" && value.charAt(value.length - 1) === "%") {
      var parsed = parseFloat(value);
      if (!isFinite(parsed) || parsed < 0) {
        throw new TypeError("bad axis");
      }
      return [2, parsed / 100];
    }
    if (typeof value !== "number" || !isFinite(value) || value < 0) {
      throw new TypeError("bad axis");
    }
    return [1, value];
  }

  function insetTag(value) {
    if (value === "auto" || value === null) {
      return [1, 0];
    }
    if (typeof value !== "number" || !isFinite(value)) {
      throw new TypeError("bad inset");
    }
    return [0, value];
  }

  function gridTag(value) {
    if (value === "auto") {
      return [0, 0];
    }
    if (!value || typeof value !== "object") {
      throw new TypeError("bad grid line");
    }
    var hasLine = value.line !== undefined;
    var hasSpan = value.span !== undefined;
    if (hasLine === hasSpan) {
      throw new TypeError("bad grid line");
    }
    var parsed = +(hasLine ? value.line : value.span);
    if (!isFinite(parsed) || parsed !== Math.trunc(parsed)) {
      throw new TypeError("bad grid line");
    }
    if (hasLine) {
      if (parsed === 0 || parsed < -32768 || parsed > 32767) {
        throw new TypeError("bad grid line");
      }
      return [1, parsed];
    }
    if (parsed < 1 || parsed > 65535) {
      throw new TypeError("bad grid line");
    }
    return [2, parsed];
  }

  function query(kind, node, a, b, text, color) {
    return core._query(kind, node || 0, a || 0, b || 0, text || "", color || [0, 0, 0, 0]);
  }

  function effect(kind, node, nums, text, color, bytes) {
    return core._effect(kind, node || 0, nums || [], text || "", color || null, bytes);
  }

  function mustEffect(result, message) {
    if (!result || !result.ok) {
      throw new Error(message);
    }
    return result;
  }

  core.column = function (gap) {
    if (arguments.length < 1) throw new Error("column(gap)");
    return create(CREATE.Column, +gap, 0, null, "", "column failed");
  };
  core.row = function (gap) {
    if (arguments.length < 1) throw new Error("row(gap)");
    return create(CREATE.Row, +gap, 0, null, "", "row failed");
  };
  core.stack = function () {
    return create(CREATE.Stack, 0, 0, null, "", "stack failed");
  };
  core.align = function (align) {
    if (arguments.length < 1) throw new Error("align(align)");
    return create(CREATE.Align, named(align, ALIGN, "align align"), 0, null, "", "align failed");
  };
  core.padding = function (edges) {
    if (arguments.length < 1) throw new Error("padding(edges)");
    return create(CREATE.Padding, +edges, 0, null, "", "padding failed");
  };
  core.pointerRegion = function () {
    return create(CREATE.Pointer, 0, 0, null, "", "pointerRegion failed");
  };
  core.decorated = function (color, radius) {
    if (arguments.length < 1) throw new Error("decorated(color)");
    return create(CREATE.Decorated, radius || 0, 0, colorOf(color), "", "decorated failed");
  };
  core.clip = function (radius) {
    if (arguments.length < 1) throw new Error("clip(radius)");
    return create(CREATE.Clip, +radius, 0, null, "", "clip failed");
  };
  core.opacity = function (alpha) {
    if (arguments.length < 1) throw new Error("opacity(alpha)");
    if (alpha < 0 || alpha > 255 || alpha !== (alpha | 0)) throw new Error("opacity alpha");
    return create(CREATE.Opacity, alpha, 0, null, "", "opacity failed");
  };
  core.scroll = function (axis) {
    return create(CREATE.Scroll, axis === undefined ? 0 : axis | 0, 0, null, "", "scroll failed");
  };
  core.transform = function () {
    return create(CREATE.Transform, 0, 0, null, "", "transform failed");
  };
  core.sized = function (width, height) {
    if (arguments.length < 2) throw new Error("sized(width, height)");
    return create(CREATE.Sized, +width, +height, null, "", "sized failed");
  };
  core.text = function (utf8, fontSize, color) {
    if (arguments.length < 3) throw new Error("text(string, fontSize, color)");
    return create(CREATE.Text, +fontSize, 0, colorOf(color), String(utf8), "text failed");
  };
  core.image = function (source, width, height, mode, required) {
    if (arguments.length < 3) throw new Error("image(source, w, h)");
    var id = create(CREATE.Image, +width, +height, null, String(source), "image failed");
    if (arguments.length >= 4 && mode !== undefined) {
      var tag = typeof mode === "number" ? mode : IMAGE_MODE[mode];
      if (tag === undefined || tag < 0 || tag > 2) throw new Error("bad image");
      var flag = arguments.length >= 5 && required ? 1 : 0;
      style(id, FIELD.ImageMode, tag, flag, 0, 0, null, "", "bad image");
    }
    return id;
  };
  core.grid = function (columns, rows, gap) {
    if (arguments.length < 3 || !columns || !rows || !columns.join || !rows.join) {
      throw new Error("grid(columns, rows, gap)");
    }
    return create(CREATE.Grid, +gap, 0, null, columns.join(",") + "|" + rows.join(","), "grid failed");
  };

  core.append = function (parent, child) {
    if (arguments.length < 2) throw new Error("append(parent, child)");
    write(OP.Insert, handle(child), handle(parent), 0, 0, 0, 0, 0, 0, null, "", "append failed");
  };
  core.insert = function (parent, child, before) {
    if (arguments.length < 3) throw new Error("insert(parent, child, before)");
    write(OP.Insert, handle(child), handle(parent), handle(before), 0, 0, 0, 0, 0, null, "", "insert failed");
  };
  core.remove = function (parent, child) {
    if (arguments.length < 2) throw new Error("remove(parent, child)");
    write(OP.Remove, handle(child), handle(parent), 0, 0, 0, 0, 0, 0, null, "", "remove failed");
  };
  core.detach = function (parent, child) {
    if (arguments.length < 2) throw new Error("detach(parent, child)");
    write(OP.Detach, handle(child), handle(parent), 0, 0, 0, 0, 0, 0, null, "", "detach failed");
  };
  core.discard = function (node) {
    if (arguments.length < 1) throw new Error("discard(node)");
    write(OP.Discard, handle(node), 0, 0, 0, 0, 0, 0, 0, null, "", "discard failed");
  };
  core.reparentChildren = function (from, to) {
    write(OP.Reparent, handle(from), handle(to), 0, 0, 0, 0, 0, 0, null, "", "reparentChildren failed");
  };

  core.setText = function (node, utf8) {
    if (arguments.length < 2) throw new Error("setText(node, string)");
    write(OP.SetText, handle(node), 0, 0, 0, 0, 0, 0, 0, null, String(utf8), "setText failed");
  };
  core.setImage = function (node, source, width, height) {
    write(OP.SetImage, handle(node), 0, 0, 0, +width, +height, 0, 0, null, String(source), "setImage failed");
  };
  core.setFontSize = function (node, size) {
    if (arguments.length < 2) throw new Error("setFontSize(node, size)");
    style(node, FIELD.FontSize, +size, 0, 0, 0, null, "", "setFontSize failed");
  };
  core.setColor = function (node, color) {
    if (arguments.length < 2) throw new Error("setColor(node, color)");
    style(node, FIELD.Color, 0, 0, 0, 0, colorOf(color), "", "setColor failed");
  };
  core.setGap = function (node, gap) {
    if (arguments.length < 2) throw new Error("setGap(node, gap)");
    style(node, FIELD.Gap, +gap, 0, 0, 0, null, "", "setGap failed");
  };
  core.setPadding = function (node, edges) {
    if (arguments.length < 2) throw new Error("setPadding(node, edges)");
    style(node, FIELD.Padding, +edges, 0, 0, 0, null, "", "setPadding failed");
  };
  core.setPaddingEdges = function (node, top, right, bottom, left) {
    style(node, FIELD.PaddingEdges, +top, +right, +bottom, +left, null, "", "setPaddingEdges failed");
  };
  core.setPaddingEdge = function (node, edge, value) {
    style(node, FIELD.PaddingEdge, edge | 0, +value, 0, 0, null, "", "setPaddingEdge failed");
  };
  core.setMargin = function (node, top, right, bottom, left) {
    style(node, FIELD.Margin, +top, +right, +bottom, +left, null, "", "setMargin failed");
  };
  core.setGrow = function (node, grow) {
    if (arguments.length < 2) throw new Error("setGrow(node, grow)");
    style(node, FIELD.Grow, +grow, 0, 0, 0, null, "", "setGrow failed");
  };
  core.setShrink = function (node, shrink) {
    style(node, FIELD.Shrink, +shrink, 0, 0, 0, null, "", "setShrink failed");
  };
  core.setBasis = function (node, basis) {
    style(node, FIELD.Basis, +basis, 0, 0, 0, null, "", "setBasis failed");
  };
  core.setMinSize = function (node, width, height) {
    style(node, FIELD.MinSize, +width, +height, 0, 0, null, "", "setMinSize failed");
  };
  core.setMaxSize = function (node, width, height) {
    style(node, FIELD.MaxSize, +width, +height, 0, 0, null, "", "setMaxSize failed");
  };
  core.setSize = function (node, width, height) {
    if (arguments.length < 3) throw new Error("setSize(node, width, height)");
    style(node, FIELD.Size, +width, +height, 0, 0, null, "", "setSize failed");
  };
  core.setAlign = function (node, align) {
    if (arguments.length < 2) throw new Error("setAlign(node, align)");
    style(node, FIELD.ContainerAlign, named(align, ALIGN, "setAlign align"), 0, 0, 0, null, "", "setAlign failed");
  };
  core.setAlignSelf = function (node, align) {
    style(node, FIELD.Align, named(align, ALIGN, "setAlignSelf align"), 0, 0, 0, null, "", "setAlignSelf failed");
  };
  core.setCross = function (node, align) {
    if (arguments.length < 2) throw new Error("setCross(node, align)");
    style(node, FIELD.Cross, named(align, CROSS, "setCross align"), 0, 0, 0, null, "", "setCross failed");
  };
  core.setJustify = function (node, justify) {
    style(node, FIELD.Justify, named(justify, JUSTIFY, "setJustify justify"), 0, 0, 0, null, "", "setJustify failed");
  };
  core.setWrap = function (node, wrap) {
    style(node, FIELD.Wrap, wrap ? 1 : 0, 0, 0, 0, null, "", "setWrap failed");
  };
  core.setFill = function (node) {
    style(node, FIELD.FlexFill, 0, 0, 0, 0, null, "", "setFill failed");
  };
  core.setReverse = function (node, reverse) {
    if (arguments.length < 2) throw new Error("setReverse(node, reverse)");
    style(node, FIELD.Reverse, reverse ? 1 : 0, 0, 0, 0, null, "", "setReverse failed");
  };
  core.setRadius = function (node, radius) {
    style(node, FIELD.Radius, +radius, 0, 0, 0, null, "", "setRadius failed");
  };
  core.setAlpha = function (node, alpha) {
    style(node, FIELD.Alpha, alpha | 0, 0, 0, 0, null, "", "setAlpha failed");
  };
  core.setStroke = function (node, width, color) {
    style(node, FIELD.Stroke, +width, 0, 0, 0, colorOf(color), "", "setStroke");
  };
  core.setMaxLines = function (node, lines) {
    style(node, FIELD.MaxLines, lines | 0, 0, 0, 0, null, "", "setMaxLines failed");
  };
  core.setEllipsis = function (node, text) {
    style(node, FIELD.Ellipsis, 0, 0, 0, 0, null, text ? String(text) : "", "setEllipsis failed");
  };
  core.setTextAlign = function (node, align) {
    style(node, FIELD.TextAlign, named(align, TEXT_ALIGN, "setTextAlign align"), 0, 0, 0, null, "", "setTextAlign failed");
  };
  core.setFontWeight = function (node, weight) {
    style(node, FIELD.Weight, weight | 0, 0, 0, 0, null, "", "setFontWeight failed");
  };
  core.setTextDirection = function (node, direction) {
    if (direction !== "ltr" && direction !== "rtl") throw new Error("setTextDirection direction");
    style(node, FIELD.Direction, direction === "rtl" ? 1 : 0, 0, 0, 0, null, "", "setTextDirection failed");
  };
  core.setLineHeight = function (node, height) {
    style(node, FIELD.LineHeight, +height, 0, 0, 0, null, "", "setLineHeight failed");
  };
  core.setFamily = function (node, family) {
    style(node, FIELD.Family, 0, 0, 0, 0, null, family ? String(family) : "", "setFamily failed");
  };
  core.setSoftWrap = function (node, wrap) {
    style(node, FIELD.SoftWrap, wrap ? 1 : 0, 0, 0, 0, null, "", "setSoftWrap failed");
  };
  core.setOverflow = function (node, overflow) {
    var code = overflow === "ellipsis" ? 1 : overflow === "visible" ? 2 : 0;
    style(node, FIELD.Overflow, code, 0, 0, 0, null, "", "setOverflow failed");
  };
  core.setScrollOffset = function (node, x, y) {
    style(node, FIELD.ScrollOffset, +x, +y, 0, 0, null, "", "setScrollOffset failed");
  };
  core.setScrollAxis = function (node, axis) {
    style(node, FIELD.ScrollAxis, axis | 0, 0, 0, 0, null, "", "setScrollAxis failed");
  };
  core.setScrollChain = function (node, chain) {
    style(node, FIELD.ScrollChain, chain ? 1 : 0, 0, 0, 0, null, "", "setScrollChain failed");
  };
  core.setImageMode = function (node, mode, required) {
    var tag = typeof mode === "number" ? mode : IMAGE_MODE[mode];
    if (tag === undefined || tag < 0 || tag > 2) throw new Error("bad image");
    style(node, FIELD.ImageMode, tag, required ? 1 : 0, 0, 0, null, "", "bad image");
  };
  core.setTransform = function (node, x, y, rotate, scale) {
    style(node, FIELD.Transform, +x, +y, +rotate, +scale, null, "", "setTransform failed");
  };
  core.setTransformComponent = function (node, component, value) {
    style(node, FIELD.TransformPart, component | 0, +value, 0, 0, null, "", "setTransformComponent failed");
  };
  core.setScaleXY = function (node, x, y) {
    style(node, FIELD.ScaleXY, +x, +y, 0, 0, null, "", "setScaleXY failed");
  };
  core.setPosition = function (node, absolute) {
    style(node, FIELD.Position, absolute ? 1 : 0, 0, 0, 0, null, "", "setPosition failed");
  };
  core.setAspectRatio = function (node, ratio) {
    if (ratio !== null && typeof ratio !== "number") throw new Error("bad aspect");
    style(node, FIELD.Aspect, ratio === null ? 0 : 1, ratio || 0, 0, 0, null, "", "setAspectRatio failed");
  };
  core.setAxisSize = function (node, width, height) {
    if (arguments.length < 3) throw new Error("setAxisSize(node, width, height)");
    var w = axisTag(width);
    var h = axisTag(height);
    style(node, FIELD.AxisSize, w[1], h[1], 0, 0, [w[0], h[0], 0, 0], "", "setAxisSize failed");
  };
  core.setMinAxis = function (node, width, height) {
    var w = axisTag(width);
    var h = axisTag(height);
    style(node, FIELD.MinAxis, w[1], h[1], 0, 0, [w[0], h[0], 0, 0], "", "setMinAxis failed");
  };
  core.setMaxAxis = function (node, width, height) {
    var w = axisTag(width);
    var h = axisTag(height);
    style(node, FIELD.MaxAxis, w[1], h[1], 0, 0, [w[0], h[0], 0, 0], "", "setMaxAxis failed");
  };
  core.setSizeAxis = function (node, axis, kind, value) {
    style(node, FIELD.SizeAxis, axis | 0, kind | 0, +value, 0, null, "", "setSizeAxis failed");
  };
  core.setInset = function (node, top, right, bottom, left) {
    var edges = [insetTag(top), insetTag(right), insetTag(bottom), insetTag(left)];
    style(node, FIELD.Inset, edges[0][1], edges[1][1], edges[2][1], edges[3][1], [edges[0][0], edges[1][0], edges[2][0], edges[3][0]], "", "setInset failed");
  };
  core.setInsetEdge = function (node, edge, value) {
    var auto = value === null || value === "auto" ? 1 : 0;
    style(node, FIELD.InsetEdge, edge | 0, auto ? 0 : +value, auto, 0, null, "", "setInsetEdge failed");
  };
  core.setGridLine = function (node, edge, kind, value) {
    style(node, FIELD.GridLine, edge | 0, kind | 0, value | 0, 0, null, "", "setGridLine failed");
  };
  core.setGridPlacement = function (node, columnStart, columnEnd, rowStart, rowEnd) {
    var lines = [gridTag(columnStart), gridTag(columnEnd), gridTag(rowStart), gridTag(rowEnd)];
    style(node, FIELD.GridLine, 0, lines[0][0], lines[0][1], 0, null, "", "setGridPlacement failed");
    style(node, FIELD.GridLine, 1, lines[1][0], lines[1][1], 0, null, "", "setGridPlacement failed");
    style(node, FIELD.GridLine, 2, lines[2][0], lines[2][1], 0, null, "", "setGridPlacement failed");
    style(node, FIELD.GridLine, 3, lines[3][0], lines[3][1], 0, null, "", "setGridPlacement failed");
  };
  core.setFocusable = function (node, enabled) {
    style(node, FIELD.Focusable, enabled ? 1 : 0, 0, 0, 0, null, "", "setFocusable failed");
  };
  core.setDragClaim = function (node, claim) {
    var code = claim === "horizontal" ? 1 : claim === "free" ? 2 : 0;
    if (claim !== "horizontal" && claim !== "free" && claim !== "none" && claim !== undefined) {
      throw new Error("drag claim");
    }
    style(node, FIELD.DragClaim, code, 0, 0, 0, null, "", "setDragClaim failed");
  };
  core.setTextLimit = function (node, limit) {
    style(node, FIELD.TextLimit, limit < 0 ? -1 : limit | 0, 0, 0, 0, null, "", "setTextLimit failed");
  };
  core.setPanel = function (node) {
    style(node, FIELD.Panel, 0, 0, 0, 0, null, "", "setPanel failed");
  };
  core.setTextScale = function (scale) {
    mustEffect(effect(EFFECT.SetTextScale, 0, [+scale]), "setTextScale failed");
  };

  function readQuery(kind, node, a, b, text, color, message) {
    var result = query(kind, node, a, b, text, color);
    if (!result || !result.ok) throw new Error(message);
    return result;
  }

  core.parent = function (node) {
    if (arguments.length < 1) throw new Error("parent(node)");
    return readQuery(QUERY.Parent, handle(node), 0, 0, "", null, "parent failed").id || 0;
  };
  core.firstChild = function (node) {
    if (arguments.length < 1) throw new Error("firstChild(node)");
    return readQuery(QUERY.FirstChild, handle(node), 0, 0, "", null, "firstChild failed").id || 0;
  };
  core.nextSibling = function (node) {
    if (arguments.length < 1) throw new Error("nextSibling(node)");
    return readQuery(QUERY.NextSibling, handle(node), 0, 0, "", null, "nextSibling failed").id || 0;
  };
  core.isText = function (node) {
    if (arguments.length < 1) throw new Error("isText(node)");
    return readQuery(QUERY.IsText, handle(node), 0, 0, "", null, "isText failed").n[0] === 1;
  };
  core.rect = function (node) {
    var result = readQuery(QUERY.Rect, handle(node), 0, 0, "", null, "rect failed");
    return { x: result.n[0], y: result.n[1], w: result.n[2], h: result.n[3] };
  };
  core.scrollOffset = function (node) {
    var result = readQuery(QUERY.ScrollOffset, handle(node), 0, 0, "", null, "scrollOffset failed");
    return { x: result.n[0], y: result.n[1] };
  };
  core.overscroll = function (node) {
    var result = readQuery(QUERY.Overscroll, handle(node), 0, 0, "", null, "overscroll failed");
    return { x: result.n[0], y: result.n[1] };
  };
  core.metrics = function () {
    var result = readQuery(QUERY.Metrics, 0, 0, 0, "", null, "metrics failed");
    return { width: result.n[0], height: result.n[1], viewInsets: { top: 0, right: 0, bottom: 0, left: 0 } };
  };
  core.measureText = function (text, fontSize, maxWidth, color) {
    var result = readQuery(QUERY.MeasureText, 0, +fontSize, +maxWidth, String(text), colorOf(color), "measureText failed");
    return { w: result.n[0], h: result.n[1] };
  };
  core.caretAt = function (node, x, y) {
    if (arguments.length < 3) throw new Error("caretAt(node, x, y)");
    var result = readQuery(QUERY.CaretAt, handle(node), +x, +y, "", null, "caretAt failed");
    return { unit: result.n[0], x: result.n[1], y: result.n[2], w: result.n[3], h: result.n[4] };
  };
  core.caretAtUnit = function (node, unit) {
    var result = readQuery(QUERY.CaretAtUnit, handle(node), unit | 0, 0, "", null, "caretAtUnit failed");
    return { x: result.n[0], y: result.n[1], w: result.n[2], h: result.n[3] };
  };
  core.wordRange = function (node, unit) {
    var result = readQuery(QUERY.WordRange, handle(node), unit | 0, 0, "", null, "wordRange failed");
    return { start: result.n[0], end: result.n[1] };
  };
  core.focused = function () {
    return readQuery(QUERY.Focused, 0, 0, 0, "", null, "focused failed").id || 0;
  };
  core.isFocusable = function (node) {
    return readQuery(QUERY.IsFocusable, handle(node), 0, 0, "", null, "isFocusable failed").n[0] === 1;
  };
  core.pointerPosition = function () {
    var result = readQuery(QUERY.Pointer, 0, 0, 0, "", null, "pointerPosition failed");
    return { x: result.n[0], y: result.n[1] };
  };
  core.textScale = function () {
    return readQuery(QUERY.TextScale, 0, 0, 0, "", null, "textScale failed").n[0];
  };
  core.reduceMotion = function () {
    return readQuery(QUERY.ReduceMotion, 0, 0, 0, "", null, "reduceMotion failed").n[0] === 1;
  };
  core.layerTreeStats = function () {
    var result = readQuery(QUERY.LayerStats, 0, 0, 0, "", null, "layerTreeStats failed");
    return { epoch: result.n[0], layers: result.n[1], pictures: result.n[2] };
  };
  core.flingTarget = function (node) {
    var result = readQuery(QUERY.FlingTarget, handle(node), 0, 0, "", null, "flingTarget");
    if (result.n[2] === 1) return null;
    return { x: result.n[0], y: result.n[1] };
  };
  core.graphemeIndex = function (text, unit) {
    return readQuery(QUERY.GraphemeIndex, 0, unit | 0, 0, String(text), null, "graphemeIndex failed").n[0];
  };
  core.graphemeCount = function (text) {
    return readQuery(QUERY.GraphemeCount, 0, 0, 0, String(text), null, "graphemeCount failed").n[0];
  };
  core.graphemeUnit = function (text, index) {
    return readQuery(QUERY.GraphemeUnit, 0, index | 0, 0, String(text), null, "graphemeUnit failed").n[0];
  };
  core.selectionRects = function (node, start, end) {
    var result = readQuery(QUERY.SelectionRects, handle(node), start | 0, end | 0, "", null, "selectionRects failed");
    var rects = [];
    var nums = result.n || [];
    for (var index = 0; index + 3 < nums.length; index += 4) {
      rects.push({ x: nums[index], y: nums[index + 1], w: nums[index + 2], h: nums[index + 3] });
    }
    return rects;
  };

  core.focus = function (node) {
    mustEffect(effect(EFFECT.Focus, handle(node), []), "focus failed");
  };
  core.blur = function () {
    mustEffect(effect(EFFECT.Blur, 0, []), "blur failed");
  };
  core.focusStep = function (direction) {
    var code = named(direction, { next: 0, previous: 1, left: 2, right: 3, up: 4, down: 5 }, "focus direction");
    mustEffect(effect(EFFECT.FocusStep, 0, [code]), "focusStep failed");
  };
  core.pushFocusScope = function (node) {
    mustEffect(effect(EFFECT.PushScope, handle(node), []), "pushFocusScope failed");
  };
  core.popFocusScope = function () {
    mustEffect(effect(EFFECT.PopScope, 0, []), "popFocusScope failed");
  };
  core.scrollTo = function (node, x, y, stiffness, damping) {
    mustEffect(effect(EFFECT.ScrollTo, handle(node), [+x, +y, +stiffness, +damping]), "scrollTo failed");
  };
  core.fling = function (node, vx, vy) {
    mustEffect(effect(EFFECT.Fling, handle(node), [+vx, +vy]), "fling failed");
  };
  core.stopScroll = function (node) {
    mustEffect(effect(EFFECT.StopScroll, handle(node), []), "stopScroll failed");
  };
  core.animate = function (node, prop, from, to, stiffness, damping) {
    mustEffect(effect(EFFECT.Animate, handle(node), [+from, +to, +stiffness, +damping], String(prop)), "animate failed");
  };
  core.scrollIntoView = function (node, edge) {
    mustEffect(effect(EFFECT.ScrollIntoView, handle(node), [edge | 0]), "scrollIntoView");
  };
  core.placePanel = function (panel, anchor, align) {
    mustEffect(effect(EFFECT.PlacePanel, handle(panel), [handle(anchor), align | 0]), "placePanel failed");
  };
  core.textEdit = function (node, op, text) {
    if (arguments.length < 2) throw new Error("textEdit(node, op, text)");
    var result = mustEffect(effect(EFFECT.TextEdit, handle(node), [op | 0], text === undefined || text === null ? "" : String(text)), "textEdit failed");
    return { text: result.t, start: result.n[0], end: result.n[1] };
  };
  core.textCommand = function (node, name, extra, recordUndo) {
    if (arguments.length < 2) throw new Error("textCommand(node, name, extra)");
    var extraText = extra === undefined || extra === null ? "" : String(extra);
    var result = mustEffect(effect(EFFECT.TextCommand, handle(node), [recordUndo ? 1 : 0], String(name) + "\0" + extraText), "textCommand failed");
    return { text: result.t, start: result.n[0], end: result.n[1] };
  };
  core.setImeEnabled = function (enabled) {
    mustEffect(effect(EFFECT.SetImeEnabled, 0, [enabled ? 1 : 0]), "setImeEnabled failed");
  };
  core.setImeText = function (text) {
    mustEffect(effect(EFFECT.SetImeText, 0, [], String(text)), "setImeText failed");
  };
  core.setImeConfig = function (keyboard, action) {
    mustEffect(effect(EFFECT.SetImeConfig, 0, [keyboard | 0, action | 0]), "setImeConfig failed");
  };
  core.setImeState = function (text, selStart, selEnd, compStart, compEnd, x, y, w, h) {
    if (text === null || text === undefined) {
      mustEffect(effect(EFFECT.SetImeState, 0, [0]), "setImeState failed");
      return;
    }
    mustEffect(effect(EFFECT.SetImeState, 0, [1, selStart | 0, selEnd | 0, compStart | 0, compEnd | 0, +x, +y, +w, +h], String(text)), "setImeState failed");
  };
  core.setSemantics = function (node, role, label, value, checked, selStart, selEnd, disabled, editable, readOnly, description, hint) {
    if (arguments.length < 2) throw new Error("setSemantics(node, role)");
    var id = handle(node);
    var orderSet = 0;
    var order = 0;
    var live = -1;
    if (hint && typeof hint === "object") {
      if (hint.order !== undefined && hint.order !== null) {
        orderSet = 1;
        order = hint.order | 0;
      }
      if (hint.live === "off") live = 0;
      else if (hint.live === "polite") live = 1;
      else if (hint.live === "assertive") live = 2;
    }
    var labelText = label ? String(label) : "";
    var valueText = value ? String(value) : "";
    var descriptionText = description ? String(description) : "";
    mustEffect(effect(EFFECT.SetSemantics, id, [role | 0, checked === undefined ? -1 : checked | 0, selStart === undefined ? -1 : selStart | 0, selEnd === undefined ? -1 : selEnd | 0, disabled === undefined ? -1 : disabled | 0, editable === undefined ? -1 : editable | 0, readOnly === undefined ? -1 : readOnly | 0, orderSet, 0, 0, 0, order, live], labelText + "\0" + valueText + "\0" + descriptionText), "setSemantics failed");
  };
  core.setRecognizers = function (node, spec) {
    if (arguments.length < 2 || !spec) throw new Error("setRecognizers(node, spec)");
    var drag = -1;
    if (spec.drag === "free") drag = 0;
    else if (spec.drag === "horizontal") drag = 1;
    else if (spec.drag === "vertical") drag = 2;
    mustEffect(effect(EFFECT.SetRecognizers, handle(node), [spec.tap ? 1 : 0, spec.longPress ? 1 : 0, drag, spec.doubleTapMs || 0]), "setRecognizers");
  };
  core.setScrollPhysics = function (node, tau, mode, stiffness, damping, factor, snapX, alignX, snapY, alignY, accept, curve, snapStiffness, snapDamping) {
    if (arguments.length < 10) {
      throw new Error("setScrollPhysics(node, tau, mode, stiffness, damping, factor, snapX, alignX, snapY, alignY)");
    }
    mustEffect(effect(EFFECT.SetScrollPhysics, handle(node), [+tau, mode | 0, +stiffness, +damping, +factor, +snapX, alignX | 0, +snapY, alignY | 0, accept || 0, curve || 0, snapStiffness || 0, snapDamping || 0]), "setScrollPhysics failed");
  };
  core.registerFont = function (family, bytes) {
    if (arguments.length < 2) throw new Error("registerFont(family, bytes)");
    var result = effect(EFFECT.RegisterFont, 0, [], String(family), null, bytes);
    if (!result || !result.ok) throw new Error("registerFont failed");
  };
  core.reload = function (source) {
    if (arguments.length < 1 || !source) throw new Error("reload(source)");
    mustEffect(effect(EFFECT.Reload, 0, [], String(source)), "reload failed");
  };
  core.measurePanel = function (node, mode, width, height) {
    var result = mustEffect(effect(EFFECT.MeasurePanel, handle(node), [mode | 0, +width, +height]), "measurePanel failed");
    return { w: result.n[0], h: result.n[1] };
  };
})(globalThis);

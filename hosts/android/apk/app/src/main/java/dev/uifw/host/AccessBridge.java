package dev.uifw.host;

import android.os.Bundle;
import android.view.View;
import android.view.accessibility.AccessibilityNodeInfo;
import android.view.accessibility.AccessibilityNodeProvider;

/**
 * Virtual nodes for the semantics tree. Arrays come from
 * {@code publishSemantics}: id, role ordinal, label, value, flags, actions,
 * scroll (offsetX, offsetY, contentW, contentH), description, parent (0 is root).
 */
public final class AccessBridge extends AccessibilityNodeProvider {
    static final int ROLE_BUTTON = 1;
    static final int ROLE_TEXT = 2;
    static final int ROLE_IMAGE = 3;
    static final int ROLE_SCROLL = 4;
    static final int ROLE_SWITCH = 5;
    static final int ROLE_TEXT_FIELD = 6;
    static final int ROLE_SEARCH_FIELD = 7;
    static final int ROLE_TEXT_AREA = 8;

    static final int ACTIVATE = 1;
    static final int SCROLL = 2;
    static final int SET_TEXT = 4;

    static final int FOCUSABLE = 1;
    static final int CHECKED = 2;
    static final int DISABLED = 4;
    static final int EDITABLE = 8;

    private long[] ids = new long[0];
    private int[] roles = new int[0];
    private String[] labels = new String[0];
    private String[] values = new String[0];
    private int[] flags = new int[0];
    private int[] actions = new int[0];
    private float[] scroll = new float[0];
    private String[] descriptions = new String[0];
    private long[] parents = new long[0];
    private View host;

    public void update(
            long[] ids,
            int[] roles,
            String[] labels,
            String[] values,
            int[] flags,
            int[] actions,
            float[] scroll,
            String[] descriptions,
            long[] parents) {
        this.ids = ids == null ? new long[0] : ids;
        this.roles = roles == null ? new int[0] : roles;
        this.labels = labels == null ? new String[0] : labels;
        this.values = values == null ? new String[0] : values;
        this.flags = flags == null ? new int[0] : flags;
        this.actions = actions == null ? new int[0] : actions;
        this.scroll = scroll == null ? new float[0] : scroll;
        this.descriptions = descriptions == null ? new String[0] : descriptions;
        this.parents = parents == null ? new long[0] : parents;
    }

    @Override
    public AccessibilityNodeInfo createAccessibilityNodeInfo(int virtualViewId) {
        View host = this.host;
        if (host == null) {
            return null;
        }
        if (virtualViewId == View.NO_ID) {
            AccessibilityNodeInfo root = AccessibilityNodeInfo.obtain(host);
            root.setClassName(host.getClass().getName());
            for (int index = 0; index < ids.length; index++) {
                if (isRootChild(index)) {
                    root.addChild(host, index);
                }
            }
            return root;
        }
        if (virtualViewId < 0 || virtualViewId >= ids.length) {
            return null;
        }
        AccessibilityNodeInfo info = AccessibilityNodeInfo.obtain(host, virtualViewId);
        info.setPackageName(host.getContext().getPackageName());
        int role = roleAt(virtualViewId);
        info.setClassName(className(role));
        String label = stringAt(labels, virtualViewId);
        String value = stringAt(values, virtualViewId);
        if (role == ROLE_IMAGE) {
            info.setContentDescription(label);
        } else if (!value.isEmpty()) {
            info.setText(value);
        } else {
            info.setText(label);
        }
        int flagBits = intAt(flags, virtualViewId);
        int actionBits = intAt(actions, virtualViewId);
        info.setFocusable((flagBits & FOCUSABLE) != 0);
        boolean clickable = (actionBits & ACTIVATE) != 0;
        info.setClickable(clickable);
        if (clickable) {
            info.addAction(AccessibilityNodeInfo.ACTION_CLICK);
        }
        if ((actionBits & SCROLL) != 0) {
            info.addAction(AccessibilityNodeInfo.ACTION_SCROLL_FORWARD);
            info.addAction(AccessibilityNodeInfo.ACTION_SCROLL_BACKWARD);
        }
        if ((actionBits & SET_TEXT) != 0) {
            info.addAction(AccessibilityNodeInfo.ACTION_SET_TEXT);
        }
        info.setEnabled((flagBits & DISABLED) == 0);
        info.setEditable((flagBits & EDITABLE) != 0);
        if (role == ROLE_SWITCH) {
            info.setCheckable(true);
            info.setChecked((flagBits & CHECKED) != 0);
        } else if ((flagBits & CHECKED) != 0) {
            info.setCheckable(true);
            info.setChecked(true);
        }
        long myId = ids[virtualViewId];
        for (int index = 0; index < ids.length; index++) {
            if (parentAt(index) == myId) {
                info.addChild(host, index);
            }
        }
        int parentIndex = indexOf(parentAt(virtualViewId));
        if (parentIndex >= 0) {
            info.setParent(host, parentIndex);
        } else {
            info.setParent(host);
        }
        return info;
    }

    @Override
    public boolean performAction(int virtualViewId, int action, Bundle arguments) {
        if (virtualViewId < 0 || virtualViewId >= ids.length) {
            return false;
        }
        int actionBits = intAt(actions, virtualViewId);
        long id = ids[virtualViewId];
        if (action == AccessibilityNodeInfo.ACTION_CLICK) {
            if ((actionBits & ACTIVATE) == 0) {
                return false;
            }
            nativeActivate(id);
            return true;
        }
        if (action == AccessibilityNodeInfo.ACTION_SCROLL_FORWARD
                || action == AccessibilityNodeInfo.ACTION_SCROLL_BACKWARD) {
            if ((actionBits & SCROLL) == 0) {
                return false;
            }
            nativeScroll(id, action == AccessibilityNodeInfo.ACTION_SCROLL_FORWARD);
            return true;
        }
        if (action == AccessibilityNodeInfo.ACTION_SET_TEXT) {
            if ((actionBits & SET_TEXT) == 0) {
                return false;
            }
            CharSequence text = arguments == null
                    ? null
                    : arguments.getCharSequence(
                            AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE);
            nativeSetText(id, text == null ? "" : text.toString());
            return true;
        }
        return false;
    }

    void attach(View host) {
        this.host = host;
    }

    private boolean isRootChild(int index) {
        return indexOf(parentAt(index)) < 0;
    }

    private long parentAt(int index) {
        if (index < 0 || index >= parents.length) {
            return 0;
        }
        return parents[index];
    }

    private int indexOf(long id) {
        if (id == 0) {
            return -1;
        }
        for (int index = 0; index < ids.length; index++) {
            if (ids[index] == id) {
                return index;
            }
        }
        return -1;
    }

    private int roleAt(int index) {
        return intAt(roles, index);
    }

    private static int intAt(int[] values, int index) {
        if (index < 0 || index >= values.length) {
            return 0;
        }
        return values[index];
    }

    private static String stringAt(String[] values, int index) {
        if (index < 0 || index >= values.length || values[index] == null) {
            return "";
        }
        return values[index];
    }

    static String className(int role) {
        return switch (role) {
            case ROLE_BUTTON -> "android.widget.Button";
            case ROLE_SWITCH -> "android.widget.Switch";
            case ROLE_TEXT_FIELD, ROLE_SEARCH_FIELD, ROLE_TEXT_AREA -> "android.widget.EditText";
            case ROLE_SCROLL -> "android.widget.ScrollView";
            case ROLE_IMAGE -> "android.widget.Image";
            case ROLE_TEXT -> "android.widget.TextView";
            default -> "android.view.View";
        };
    }

    private static native void nativeActivate(long id);

    private static native void nativeScroll(long id, boolean forward);

    private static native void nativeSetText(long id, String text);
}

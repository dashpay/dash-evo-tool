import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { renderHook } from "@testing-library/react";
import { useKeyboardShortcuts } from "./useKeyboardShortcuts";

describe("useKeyboardShortcuts", () => {
  let addEventListenerSpy: ReturnType<typeof vi.spyOn>;
  let removeEventListenerSpy: ReturnType<typeof vi.spyOn>;

  beforeEach(() => {
    addEventListenerSpy = vi.spyOn(window, "addEventListener");
    removeEventListenerSpy = vi.spyOn(window, "removeEventListener");
  });

  afterEach(() => {
    addEventListenerSpy.mockRestore();
    removeEventListenerSpy.mockRestore();
  });

  it("registers keydown listener on mount", () => {
    const action = vi.fn();
    renderHook(() =>
      useKeyboardShortcuts([{ key: "1", description: "test", action }]),
    );
    expect(addEventListenerSpy).toHaveBeenCalledWith(
      "keydown",
      expect.any(Function),
    );
  });

  it("removes keydown listener on unmount", () => {
    const action = vi.fn();
    const { unmount } = renderHook(() =>
      useKeyboardShortcuts([{ key: "1", description: "test", action }]),
    );
    unmount();
    expect(removeEventListenerSpy).toHaveBeenCalledWith(
      "keydown",
      expect.any(Function),
    );
  });

  it("fires action when matching key is pressed", () => {
    const action = vi.fn();
    renderHook(() =>
      useKeyboardShortcuts([{ key: "1", description: "test", action }]),
    );
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "1" }));
    expect(action).toHaveBeenCalledTimes(1);
  });

  it("does not fire action for non-matching key", () => {
    const action = vi.fn();
    renderHook(() =>
      useKeyboardShortcuts([{ key: "1", description: "test", action }]),
    );
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "2" }));
    expect(action).not.toHaveBeenCalled();
  });

  it("does not fire when typing in an input element", () => {
    const action = vi.fn();
    renderHook(() =>
      useKeyboardShortcuts([{ key: "1", description: "test", action }]),
    );

    const input = document.createElement("input");
    document.body.appendChild(input);
    input.focus();

    // Dispatch from the element so e.target is the input (bubbles to window)
    input.dispatchEvent(
      new KeyboardEvent("keydown", { key: "1", bubbles: true }),
    );
    expect(action).not.toHaveBeenCalled();

    document.body.removeChild(input);
  });

  it("does not fire when typing in a textarea element", () => {
    const action = vi.fn();
    renderHook(() =>
      useKeyboardShortcuts([{ key: "1", description: "test", action }]),
    );

    const textarea = document.createElement("textarea");
    document.body.appendChild(textarea);
    textarea.focus();

    // Dispatch from the element so e.target is the textarea (bubbles to window)
    textarea.dispatchEvent(
      new KeyboardEvent("keydown", { key: "1", bubbles: true }),
    );
    expect(action).not.toHaveBeenCalled();

    document.body.removeChild(textarea);
  });

  it("does not fire when typing in a contenteditable element", () => {
    const action = vi.fn();
    renderHook(() =>
      useKeyboardShortcuts([{ key: "1", description: "test", action }]),
    );

    const div = document.createElement("div");
    // Use setAttribute because jsdom doesn't reflect contentEditable property to attribute
    div.setAttribute("contenteditable", "true");
    document.body.appendChild(div);
    div.focus();

    // Dispatch from the element itself so e.target is the contenteditable div
    div.dispatchEvent(
      new KeyboardEvent("keydown", { key: "1", bubbles: true }),
    );
    expect(action).not.toHaveBeenCalled();

    document.body.removeChild(div);
  });

  it("supports multiple shortcuts", () => {
    const action1 = vi.fn();
    const action2 = vi.fn();
    renderHook(() =>
      useKeyboardShortcuts([
        { key: "1", description: "first", action: action1 },
        { key: "2", description: "second", action: action2 },
      ]),
    );
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "2" }));
    expect(action1).not.toHaveBeenCalled();
    expect(action2).toHaveBeenCalledTimes(1);
  });

  it("prevents default on matching key", () => {
    const action = vi.fn();
    renderHook(() =>
      useKeyboardShortcuts([{ key: "1", description: "test", action }]),
    );
    const event = new KeyboardEvent("keydown", { key: "1" });
    const preventDefaultSpy = vi.spyOn(event, "preventDefault");
    window.dispatchEvent(event);
    expect(preventDefaultSpy).toHaveBeenCalled();
  });
});

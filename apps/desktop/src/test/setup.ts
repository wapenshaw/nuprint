import "@testing-library/jest-dom/vitest";
import { cleanup } from "@testing-library/react";
import { afterEach } from "vitest";
import { setBackend } from "../api";

afterEach(() => {
  cleanup();
  setBackend(null);
});

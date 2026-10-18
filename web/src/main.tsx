import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { AppRouter } from "./router";
import "./styles.css";

const queryClient = new QueryClient();
const root = document.getElementById("root");

if (!root) {
  throw new Error("root element is missing");
}

createRoot(root).render(
  <StrictMode>
    <QueryClientProvider client={queryClient}>
      <AppRouter />
    </QueryClientProvider>
  </StrictMode>,
);

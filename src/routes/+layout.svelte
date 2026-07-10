<script lang="ts">
  import "../app.css";
  import { Toaster } from "svelte-sonner";

  let { children } = $props();
</script>

{@render children()}

<Toaster
  richColors
  theme="dark"
  duration={10000}
  toastOptions={{
    classes: {
      title: "text-base font-bold!",
      description: "text-base",
    },
  }}
/>

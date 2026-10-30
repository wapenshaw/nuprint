// Points the hero's download button at the visitor's system. Windows is the default
// in the markup; a Mac has two builds, so it goes to the tickets instead of guessing.
(() => {
  const button = document.getElementById("primary-download");
  const other = document.getElementById("other-systems");
  if (!button || !other) return;

  const platform = (navigator.userAgentData?.platform || navigator.platform || "").toLowerCase();
  const agent = navigator.userAgent.toLowerCase();
  const isMac = platform.startsWith("mac") || (agent.includes("mac os") && !agent.includes("iphone") && !agent.includes("ipad"));
  const isLinux = !isMac && (platform.includes("linux") || agent.includes("linux")) && !agent.includes("android");

  const label = button.querySelector("span");
  if (isMac) {
    button.href = button.dataset.mac;
    label.textContent = "Download for Mac";
    other.textContent = "Windows and Linux";
  } else if (isLinux) {
    button.href = button.dataset.linux;
    label.textContent = "Download for Linux";
    other.textContent = "Windows and macOS";
  }
})();

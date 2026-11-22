// ZeroRec Apple-Style Interactive Showcase & Telemetry Engine

document.addEventListener("DOMContentLoaded", () => {
  // 1. Embedded Benchmark Telemetry Dataset
  const BENCH_DATA = {
    "1080p": {
      zerorec: {
        name: "ZeroRec (1080p60 H.264)",
        mean_cpu: 6.4,
        peak_cpu: 7.9,
        max_rss: 54.4,
        frames: 600,
        color: "#2997ff"
      },
      ffmpeg: {
        name: "FFmpeg (1080p60 H.264)",
        mean_cpu: 81.6,
        peak_cpu: 89.1,
        max_rss: 141.2,
        frames: 600,
        color: "#ff9f0a"
      },
      advantage_cpu: "12.7× Lower CPU",
      advantage_ram: "2.6× Less RAM",
      max_chart_cpu: 100
    },
    "4k": {
      zerorec: {
        name: "ZeroRec (4K60 HEVC)",
        mean_cpu: 16.4,
        peak_cpu: 19.0,
        max_rss: 99.2,
        frames: 600,
        color: "#2997ff"
      },
      ffmpeg: {
        name: "FFmpeg (4K60 HEVC)",
        mean_cpu: 215.0,
        peak_cpu: 261.6,
        max_rss: 707.7,
        frames: 600,
        color: "#ff9f0a"
      },
      advantage_cpu: "13.1× Lower CPU",
      advantage_ram: "7.1× Less RAM",
      max_chart_cpu: 280
    }
  };

  // High-fidelity telemetry timeline curves (from real bench_results.json)
  const TELEMETRY_CURVES = {
    "1080p": {
      zerorec: [
        {t:0.0, cpu:0.0}, {t:0.3, cpu:0.1}, {t:0.5, cpu:5.8}, {t:0.8, cpu:4.5}, {t:1.1, cpu:7.3},
        {t:1.5, cpu:6.7}, {t:2.0, cpu:6.3}, {t:2.5, cpu:7.1}, {t:3.0, cpu:6.8}, {t:3.2, cpu:7.9},
        {t:3.8, cpu:7.2}, {t:4.5, cpu:7.1}, {t:5.0, cpu:6.2}, {t:5.8, cpu:7.3}, {t:6.5, cpu:6.6},
        {t:7.2, cpu:6.9}, {t:8.0, cpu:7.0}, {t:8.7, cpu:7.3}, {t:9.5, cpu:7.3}, {t:10.0, cpu:6.6}
      ],
      ffmpeg: [
        {t:0.0, cpu:0.0}, {t:0.3, cpu:38.2}, {t:0.6, cpu:69.8}, {t:0.9, cpu:80.5}, {t:1.2, cpu:81.9},
        {t:1.6, cpu:89.1}, {t:2.0, cpu:85.7}, {t:2.5, cpu:81.9}, {t:3.0, cpu:81.5}, {t:3.7, cpu:86.1},
        {t:4.2, cpu:86.6}, {t:4.8, cpu:85.4}, {t:5.5, cpu:86.3}, {t:6.0, cpu:87.0}, {t:6.5, cpu:87.2},
        {t:7.2, cpu:85.1}, {t:8.0, cpu:83.6}, {t:8.5, cpu:86.9}, {t:9.2, cpu:82.3}, {t:10.0, cpu:86.0}
      ]
    },
    "4k": {
      zerorec: [
        {t:0.0, cpu:0.0}, {t:0.3, cpu:10.6}, {t:0.6, cpu:2.6}, {t:1.0, cpu:7.7}, {t:1.3, cpu:14.4},
        {t:1.7, cpu:18.4}, {t:2.2, cpu:17.5}, {t:2.8, cpu:18.9}, {t:3.3, cpu:18.6}, {t:4.0, cpu:15.6},
        {t:4.5, cpu:17.8}, {t:5.2, cpu:17.7}, {t:5.8, cpu:18.7}, {t:6.5, cpu:18.2}, {t:7.0, cpu:19.0},
        {t:7.5, cpu:18.6}, {t:8.2, cpu:17.5}, {t:8.8, cpu:18.2}, {t:9.5, cpu:18.5}, {t:10.0, cpu:17.8}
      ],
      ffmpeg: [
        {t:0.0, cpu:0.0}, {t:0.3, cpu:63.1}, {t:0.5, cpu:238.4}, {t:0.7, cpu:261.6}, {t:1.1, cpu:258.1},
        {t:1.6, cpu:245.4}, {t:2.1, cpu:245.5}, {t:2.7, cpu:224.1}, {t:3.2, cpu:227.4}, {t:3.8, cpu:217.8},
        {t:4.3, cpu:218.3}, {t:5.0, cpu:225.5}, {t:5.6, cpu:229.0}, {t:6.3, cpu:230.2}, {t:7.0, cpu:226.9},
        {t:7.5, cpu:231.6}, {t:8.2, cpu:227.7}, {t:8.7, cpu:221.3}, {t:9.4, cpu:224.1}, {t:10.0, cpu:228.1}
      ]
    }
  };

  // 2. DOM Elements Selection
  const segmentBtns = document.querySelectorAll(".segment-btn");
  const segmentSlider = document.querySelector(".segment-pill-slider");
  const cpuAdvantageBadge = document.getElementById("cpuAdvantageBadge");
  const ramAdvantageBadge = document.getElementById("ramAdvantageBadge");
  const zerorecCpuLabel = document.getElementById("zerorecCpuLabel");
  const ffmpegCpuLabel = document.getElementById("ffmpegCpuLabel");
  const zerorecCpuBar = document.getElementById("zerorecCpuBar");
  const ffmpegCpuBar = document.getElementById("ffmpegCpuBar");
  const zerorecRamLabel = document.getElementById("zerorecRamLabel");
  const ffmpegRamLabel = document.getElementById("ffmpegRamLabel");
  const zerorecRamBar = document.getElementById("zerorecRamBar");
  const ffmpegRamBar = document.getElementById("ffmpegRamBar");

  const canvas = document.getElementById("telemetryChart");
  const ctx = canvas.getContext("2d");
  const playBtn = document.getElementById("playBtn");
  const playIcon = document.getElementById("playIcon");
  const playBtnText = document.getElementById("playBtnText");
  const timelineCounter = document.getElementById("timelineCounter");
  const timelineSlider = document.getElementById("timelineSlider");

  // State
  let currentRes = "1080p";
  let isPlaying = false;
  let playhead = 10.0; // Current timestamp in seconds
  let animFrameId = null;
  let lastTimestamp = null;

  // 3. Apple Segmented Control Handler
  function updateSegmentSlider(activeBtn) {
    if (!activeBtn || !segmentSlider) return;
    const offsetLeft = activeBtn.offsetLeft;
    const width = activeBtn.offsetWidth;
    segmentSlider.style.transform = `translateX(${offsetLeft - 3}px)`;
    segmentSlider.style.width = `${width}px`;
  }

  segmentBtns.forEach(btn => {
    btn.addEventListener("click", () => {
      segmentBtns.forEach(b => {
        b.classList.remove("active");
        b.setAttribute("aria-selected", "false");
      });
      btn.classList.add("active");
      btn.setAttribute("aria-selected", "true");
      currentRes = btn.getAttribute("data-res");
      updateSegmentSlider(btn);
      updateMetricsUI();
      drawChart();
    });
  });

  // Initial position for segmented control slider
  const initialActive = document.querySelector(".segment-btn.active");
  if (initialActive) {
    setTimeout(() => updateSegmentSlider(initialActive), 50);
    window.addEventListener("resize", () => {
      const active = document.querySelector(".segment-btn.active");
      if (active) updateSegmentSlider(active);
    });
  }

  // 4. Update Metrics UI Bars and Cards
  function updateMetricsUI() {
    const data = BENCH_DATA[currentRes];
    if (!data) return;

    cpuAdvantageBadge.textContent = data.advantage_cpu;
    ramAdvantageBadge.textContent = data.advantage_ram;

    zerorecCpuLabel.textContent = `${data.zerorec.mean_cpu.toFixed(1)} %`;
    ffmpegCpuLabel.textContent = `${data.ffmpeg.mean_cpu.toFixed(1)} %`;

    // Max CPU scale for relative bar widths
    const maxCpu = currentRes === "1080p" ? 100 : 260;
    const zCpuPercent = Math.min(100, Math.max(5, (data.zerorec.mean_cpu / maxCpu) * 100));
    const fCpuPercent = Math.min(100, Math.max(5, (data.ffmpeg.mean_cpu / maxCpu) * 100));
    zerorecCpuBar.style.width = `${zCpuPercent}%`;
    ffmpegCpuBar.style.width = `${fCpuPercent}%`;

    // RAM bars
    const maxRam = currentRes === "1080p" ? 160 : 750;
    zerorecRamLabel.textContent = `${data.zerorec.max_rss.toFixed(1)} MB`;
    ffmpegRamLabel.textContent = `${data.ffmpeg.max_rss.toFixed(1)} MB`;
    const zRamPercent = Math.min(100, Math.max(5, (data.zerorec.max_rss / maxRam) * 100));
    const fRamPercent = Math.min(100, Math.max(5, (data.ffmpeg.max_rss / maxRam) * 100));
    zerorecRamBar.style.width = `${zRamPercent}%`;
    ffmpegRamBar.style.width = `${fRamPercent}%`;
  }

  // 5. Canvas High-DPI & Chart Rendering
  function resizeCanvas() {
    if (!canvas) return;
    const dpr = window.devicePixelRatio || 1;
    const rect = canvas.getBoundingClientRect();
    if (rect.width === 0) return;

    canvas.width = Math.floor(rect.width * dpr);
    canvas.height = Math.floor(rect.height * dpr);
    ctx.resetTransform();
    ctx.scale(dpr, dpr);
    drawChart();
  }
  window.addEventListener("resize", resizeCanvas);

  function drawChart() {
    if (!canvas) return;
    const rect = canvas.getBoundingClientRect();
    const width = rect.width;
    const height = rect.height;
    if (width === 0 || height === 0) return;

    ctx.clearRect(0, 0, width, height);

    const padding = { top: 22, right: 34, bottom: 26, left: 48 };
    const chartW = width - padding.left - padding.right;
    const chartH = height - padding.top - padding.bottom;
    const maxVal = BENCH_DATA[currentRes].max_chart_cpu;

    // Draw horizontal grid lines & labels
    ctx.strokeStyle = "rgba(255, 255, 255, 0.07)";
    ctx.lineWidth = 1;
    ctx.fillStyle = "rgba(134, 134, 139, 0.8)";
    ctx.font = "11px -apple-system, BlinkMacSystemFont, sans-serif";

    const steps = 4;
    for (let i = 0; i <= steps; i++) {
      const y = padding.top + (chartH / steps) * i;
      const val = Math.round(maxVal - (maxVal / steps) * i);
      ctx.beginPath();
      ctx.moveTo(padding.left, y);
      ctx.lineTo(width - padding.right, y);
      ctx.stroke();

      ctx.textAlign = "right";
      ctx.fillText(`${val}%`, padding.left - 8, y + 4);
    }

    // Time X labels (0s, 2.5s, 5s, 7.5s, 10s)
    const timeMarks = [0, 2.5, 5.0, 7.5, 10.0];
    timeMarks.forEach(sec => {
      const x = padding.left + (chartW / 10.0) * sec;
      ctx.beginPath();
      ctx.moveTo(x, padding.top);
      ctx.lineTo(x, height - padding.bottom);
      ctx.stroke();

      ctx.textAlign = "center";
      ctx.fillText(`${sec}s`, x, height - padding.bottom + 16);
    });

    const dataset = TELEMETRY_CURVES[currentRes];
    if (!dataset) return;

    // Render smooth curve helper
    function renderCurve(points, strokeColor, glowColor) {
      const visiblePts = points.filter(p => p.t <= playhead);
      if (visiblePts.length < 2) return;

      ctx.save();
      ctx.beginPath();
      visiblePts.forEach((p, idx) => {
        const x = padding.left + (Math.min(p.t, 10.0) / 10.0) * chartW;
        const y = padding.top + chartH - (p.cpu / maxVal) * chartH;
        if (idx === 0) ctx.moveTo(x, y);
        else ctx.lineTo(x, y);
      });

      ctx.strokeStyle = strokeColor;
      ctx.lineWidth = 2.5;
      ctx.stroke();

      // Soft gradient under the curve
      const last = visiblePts[visiblePts.length - 1];
      const lastX = padding.left + (Math.min(last.t, 10.0) / 10.0) * chartW;
      const firstX = padding.left + (Math.min(visiblePts[0].t, 10.0) / 10.0) * chartW;
      ctx.lineTo(lastX, padding.top + chartH);
      ctx.lineTo(firstX, padding.top + chartH);
      ctx.closePath();

      const grad = ctx.createLinearGradient(0, padding.top, 0, padding.top + chartH);
      grad.addColorStop(0, glowColor);
      grad.addColorStop(1, "rgba(0, 0, 0, 0)");
      ctx.fillStyle = grad;
      ctx.fill();

      // Current playhead dot
      ctx.beginPath();
      const dotY = padding.top + chartH - (last.cpu / maxVal) * chartH;
      ctx.arc(lastX, dotY, 4.5, 0, Math.PI * 2);
      ctx.fillStyle = strokeColor;
      ctx.shadowColor = strokeColor;
      ctx.shadowBlur = 10;
      ctx.fill();
      ctx.restore();
    }

    // Draw FFmpeg curve (orange)
    renderCurve(dataset.ffmpeg, "#ff9f0a", "rgba(255, 159, 10, 0.18)");
    // Draw ZeroRec curve (Apple Blue)
    renderCurve(dataset.zerorec, "#2997ff", "rgba(41, 151, 255, 0.22)");

    // Draw vertical playhead line
    const playheadX = padding.left + (Math.min(playhead, 10.0) / 10.0) * chartW;
    ctx.beginPath();
    ctx.strokeStyle = "rgba(255, 255, 255, 0.5)";
    ctx.lineWidth = 1;
    ctx.setLineDash([4, 4]);
    ctx.moveTo(playheadX, padding.top);
    ctx.lineTo(playheadX, height - padding.bottom);
    ctx.stroke();
    ctx.setLineDash([]);
  }

  // 6. Play / Pause Telemetry Animation
  function animate(time) {
    if (!lastTimestamp) lastTimestamp = time;
    const dt = (time - lastTimestamp) / 1000;
    lastTimestamp = time;

    playhead += dt * 1.5; // Smooth 1.5x pacing
    if (playhead >= 10.0) {
      playhead = 10.0;
      isPlaying = false;
      playIcon.innerHTML = '<polygon points="5 3 19 12 5 21 5 3"></polygon>';
      playBtnText.textContent = "Replay";
      timelineCounter.textContent = "10.0s";
      timelineSlider.value = "10";
      drawChart();
      return;
    }

    timelineCounter.textContent = `${playhead.toFixed(1)}s`;
    timelineSlider.value = playhead.toFixed(1);
    drawChart();

    if (isPlaying) {
      animFrameId = requestAnimationFrame(animate);
    }
  }

  playBtn.addEventListener("click", () => {
    if (isPlaying) {
      isPlaying = false;
      cancelAnimationFrame(animFrameId);
      playIcon.innerHTML = '<polygon points="5 3 19 12 5 21 5 3"></polygon>';
      playBtnText.textContent = "Play";
    } else {
      if (playhead >= 10.0) playhead = 0.0;
      isPlaying = true;
      lastTimestamp = null;
      playIcon.innerHTML = '<rect x="6" y="4" width="4" height="16"></rect><rect x="14" y="4" width="4" height="16"></rect>';
      playBtnText.textContent = "Pause";
      animFrameId = requestAnimationFrame(animate);
    }
  });

  // Slider Scrubbing
  timelineSlider.addEventListener("input", (e) => {
    if (isPlaying) {
      isPlaying = false;
      cancelAnimationFrame(animFrameId);
      playIcon.innerHTML = '<polygon points="5 3 19 12 5 21 5 3"></polygon>';
      playBtnText.textContent = "Play";
    }
    playhead = parseFloat(e.target.value);
    timelineCounter.textContent = `${playhead.toFixed(1)}s`;
    drawChart();
  });

  // Canvas Click / Drag Scrubbing
  let isDragging = false;
  function handleCanvasScrub(e) {
    const rect = canvas.getBoundingClientRect();
    const x = e.clientX - rect.left;
    const padding = { left: 48, right: 34 };
    const chartW = rect.width - padding.left - padding.right;

    if (x >= padding.left && x <= rect.width - padding.right) {
      const sec = ((x - padding.left) / chartW) * 10.0;
      playhead = Math.max(0.0, Math.min(10.0, sec));
      timelineCounter.textContent = `${playhead.toFixed(1)}s`;
      timelineSlider.value = playhead.toFixed(1);
      drawChart();
    }
  }

  canvas.addEventListener("mousedown", (e) => {
    isDragging = true;
    if (isPlaying) {
      isPlaying = false;
      cancelAnimationFrame(animFrameId);
      playIcon.innerHTML = '<polygon points="5 3 19 12 5 21 5 3"></polygon>';
      playBtnText.textContent = "Play";
    }
    handleCanvasScrub(e);
  });

  window.addEventListener("mousemove", (e) => {
    if (isDragging) handleCanvasScrub(e);
  });

  window.addEventListener("mouseup", () => {
    isDragging = false;
  });

  // 7. Interactive Architecture Pipeline Nodes
  const STAGE_DETAILS = {
    "zerorec-sck": {
      title: "ScreenCaptureKit (macOS Display Server)",
      desc: "WindowServer renders frames directly into hardware framebuffers. ZeroRec receives an IOSurface pointer with zero memory copies into application heap.",
      bandwidth: "0.00 GB/s",
      color: "var(--apple-blue)"
    },
    "zerorec-iosurface": {
      title: "IOSurface Shared Hardware Memory",
      desc: "Apple Silicon unified memory architecture enables CVPixelBuffers to share identical physical DRAM addresses between the GPU, ScreenCaptureKit, and VideoToolbox hardware encoder. Zero CPU memory copies occur.",
      bandwidth: "0.00 GB/s",
      color: "var(--apple-blue)"
    },
    "zerorec-lanczos": {
      title: "Metal Performance Shaders (MPS Lanczos)",
      desc: "Metal compute shaders execute high-tap Lanczos windowed sinc downsampling directly in GPU registers, delivering pristine 4K-to-1080p scaling without touching host CPU memory.",
      bandwidth: "0.00 GB/s",
      color: "var(--apple-cyan)"
    },
    "zerorec-vt": {
      title: "VideoToolbox Hardware ASIC Encoder",
      desc: "The dedicated on-die Apple Silicon Media Engine encodes H.264 or HEVC directly from the IOSurface. The CPU simply dispatches 64-bit frame descriptors with zero encoding workload.",
      bandwidth: "0.00 GB/s",
      color: "var(--apple-blue)"
    },
    "zerorec-output": {
      title: "Direct Bitstream I/O (MP4 / WHIP / SRT)",
      desc: "ZeroRec streams compressed NAL packets directly to disk or network (WHIP/WebRTC and SRT) with microsecond Mach absolute time deadline pacing.",
      bandwidth: "< 0.01 GB/s",
      color: "var(--apple-green)"
    },
    "ffmpeg-source": {
      title: "AVFoundation Capture Ingestion",
      desc: "Raw video frames are ingested into FFmpeg's AVFrame structures, allocating heap descriptors and staging memory in the process memory space.",
      bandwidth: "0.50 GB/s",
      color: "var(--apple-orange)"
    },
    "ffmpeg-ram": {
      title: "Host RAM Buffer Copy (Copy #1)",
      desc: "Uncompressed frames (e.g. 3840x2160 RGBA at 60 fps) are copied into host RAM buffers. A single 4K60 stream demands over 1.98 GB/s of continuous memory bus bandwidth.",
      bandwidth: "1.98 GB/s",
      color: "var(--apple-orange)"
    },
    "ffmpeg-swscale": {
      title: "CPU Pixel Format Conversion (Copy #2)",
      desc: "libswscale iterates through every single pixel on the CPU to convert RGBA into NV12 color format, destroying CPU cache lines and saturating memory controllers.",
      bandwidth: "1.98 GB/s",
      color: "var(--apple-orange)"
    },
    "ffmpeg-gpu": {
      title: "DMA VideoToolbox Upload (Copy #3)",
      desc: "Converted NV12 buffers must be copied and marshalled into VideoToolbox CVPixelBuffers before hardware encoding can begin.",
      bandwidth: "0.99 GB/s",
      color: "var(--apple-orange)"
    },
    "ffmpeg-muxer": {
      title: "FFmpeg Interleaved Muxer",
      desc: "Packets are queued in intermediate libavformat ring buffers with software interleaving overhead.",
      bandwidth: "0.05 GB/s",
      color: "var(--apple-orange)"
    }
  };

  const stageDetailTitle = document.getElementById("stageDetailTitle");
  const stageDetailDesc = document.getElementById("stageDetailDesc");
  const stageBandwidthVal = document.getElementById("stageBandwidthVal");
  const pipelineNodes = document.querySelectorAll(".pipeline-node");

  pipelineNodes.forEach(node => {
    node.addEventListener("click", () => {
      pipelineNodes.forEach(n => n.classList.remove("active"));
      node.classList.add("active");

      const stageKey = node.getAttribute("data-stage");
      const info = STAGE_DETAILS[stageKey];
      if (info && stageDetailTitle && stageDetailDesc && stageBandwidthVal) {
        stageDetailTitle.style.color = info.color;
        stageDetailTitle.innerHTML = `
          <span class="inspector-icon"></span>
          <span>${info.title}</span>
        `;
        stageDetailDesc.textContent = info.desc;
        stageBandwidthVal.textContent = info.bandwidth;
        stageBandwidthVal.style.color = info.bandwidth === "0.00 GB/s" ? "var(--apple-green)" : "var(--apple-orange)";
      }
    });

    node.addEventListener("keydown", (e) => {
      if (e.key === "Enter" || e.key === " ") {
        e.preventDefault();
        node.click();
      }
    });
  });

  // 8. Terminal Copy Buttons
  function copyTextToClipboard(text, btnElement, successMsg = "Copied!") {
    const originalText = btnElement.innerText || btnElement.textContent;
    if (navigator.clipboard && navigator.clipboard.writeText) {
      navigator.clipboard.writeText(text).then(() => {
        btnElement.innerText = successMsg;
        btnElement.style.color = "var(--apple-green)";
        setTimeout(() => {
          btnElement.innerText = originalText;
          btnElement.style.color = "";
        }, 1800);
      });
    } else {
      // Fallback
      const ta = document.createElement("textarea");
      ta.value = text;
      document.body.appendChild(ta);
      ta.select();
      document.execCommand("copy");
      document.body.removeChild(ta);
      btnElement.innerText = successMsg;
      setTimeout(() => { btnElement.innerText = originalText; }, 1800);
    }
  }

  const copyAllBtn = document.getElementById("copyAllBtn");
  if (copyAllBtn) {
    copyAllBtn.addEventListener("click", () => {
      const fullCommands = `cargo build --release\npython3 scripts/bench_compare.py\nREC_NO_DISCLAIM=1 ./target/release/zerorec record --test-pattern -r 1080p --fps 60 --duration 10 -o /tmp/zerorec.mp4`;
      copyTextToClipboard(fullCommands, copyAllBtn, "Copied All!");
    });
  }

  const lineCopyBtns = document.querySelectorAll(".line-copy-btn");
  lineCopyBtns.forEach(btn => {
    btn.addEventListener("click", () => {
      const textToCopy = btn.getAttribute("data-copy");
      if (textToCopy) {
        copyTextToClipboard(textToCopy, btn, "Copied");
      }
    });
  });

  // 9. Initialize Page State
  updateMetricsUI();
  resizeCanvas();
});

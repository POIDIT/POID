"""The M06 DoD fixture: pandas + matplotlib, offline, no pip."""

import io
import base64

import matplotlib

matplotlib.use("Agg")

import matplotlib.pyplot as plt
import pandas as pd

df = pd.DataFrame({"month": ["Jan", "Feb", "Mar"], "sales": [3, 7, 5]})
total = int(df["sales"].sum())

fig, ax = plt.subplots(figsize=(3, 2))
ax.bar(df["month"], df["sales"])
buf = io.BytesIO()
fig.savefig(buf, format="png")
png_b64 = base64.b64encode(buf.getvalue()).decode("ascii")

RESULT = {"total": total, "png_prefix": png_b64[:8], "png_len": len(png_b64)}

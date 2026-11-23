"""Screenshot checks for the render spike: pixel spread and change detection."""
import sys
from PIL import Image, ImageChops, ImageStat

if sys.argv[1] == "spread":
    print(f"{ImageStat.Stat(Image.open(sys.argv[2]).convert('L')).stddev[0]:.2f}")
elif sys.argv[1] == "differ":
    a = Image.open(sys.argv[2]).convert("RGB")
    b = Image.open(sys.argv[3]).convert("RGB")
    sys.exit(0 if ImageChops.difference(a, b).getbbox() else 1)

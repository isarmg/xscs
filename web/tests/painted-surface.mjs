// Corner rendering moves an opaque surface background into its SVG border-image.
// Read that actual paint layer; the host background is intentionally transparent.
export async function paintedSurfaceColor(locator) {
  return locator.evaluate(async element => {
    const css = getComputedStyle(element);
    const state = element.getAttribute("data-xcss-server-corner-state");
    if (state !== "paint" && state !== "clip") return css.backgroundColor;
    if (!css.borderImageSlice.split(/\s+/).includes("fill")) {
      throw new Error("Rendered surface has no border-image fill");
    }
    const source = /^url\((["']?)(.*?)\1\)$/.exec(css.borderImageSource)?.[2];
    if (!source?.startsWith("blob:")) {
      throw new Error("Rendered surface has no Blob SVG paint layer");
    }
    const image = new Image();
    await new Promise((resolve, reject) => {
      image.onload = resolve;
      image.onerror = () => reject(new Error("Surface paint image failed to load"));
      image.src = source;
    });
    if (image.naturalWidth < 2 || image.naturalHeight < 2) {
      throw new Error("Surface paint image has no interior");
    }
    const canvas = document.createElement("canvas");
    canvas.width = canvas.height = 1;
    const context = canvas.getContext("2d");
    // Copy one interior source pixel, without averaging transparent corner pixels.
    context.drawImage(image, Math.floor(image.naturalWidth / 2), Math.floor(image.naturalHeight / 2), 1, 1, 0, 0, 1, 1);
    const [red, green, blue, alpha] = context.getImageData(0, 0, 1, 1).data;
    if (alpha !== 255) throw new Error(`Surface paint is not opaque: ${alpha}`);
    return `rgb(${red}, ${green}, ${blue})`;
  });
}

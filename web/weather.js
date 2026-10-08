// Generated from web/weather.ts by pnpm run generate. Do not edit.
/*! openapi-fetch
MIT License

Copyright (c) 2023 Drew Powers

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
*/
"use strict";
(() => {
  // node_modules/openapi-fetch/dist/index.mjs
  var PATH_PARAM_RE = /\{[^{}]+\}/g;
  var supportsRequestInitExt = () => {
    return typeof process === "object" && Number.parseInt(process?.versions?.node?.substring(0, 2)) >= 18 && process.versions.undici;
  };
  function randomID() {
    return Math.random().toString(36).slice(2, 11);
  }
  function createClient(clientOptions) {
    let {
      baseUrl = "",
      Request: CustomRequest = globalThis.Request,
      fetch: baseFetch = globalThis.fetch,
      querySerializer: globalQuerySerializer,
      bodySerializer: globalBodySerializer,
      pathSerializer: globalPathSerializer,
      headers: baseHeaders,
      requestInitExt = void 0,
      ...baseOptions
    } = { ...clientOptions };
    requestInitExt = supportsRequestInitExt() ? requestInitExt : void 0;
    baseUrl = removeTrailingSlash(baseUrl);
    const globalMiddlewares = [];
    async function coreFetch(schemaPath, fetchOptions) {
      const {
        baseUrl: localBaseUrl,
        fetch: fetch2 = baseFetch,
        Request = CustomRequest,
        headers,
        params = {},
        parseAs = "json",
        querySerializer: requestQuerySerializer,
        bodySerializer = globalBodySerializer ?? defaultBodySerializer,
        pathSerializer: requestPathSerializer,
        body,
        middleware: requestMiddlewares = [],
        ...init
      } = fetchOptions || {};
      let finalBaseUrl = baseUrl;
      if (localBaseUrl) {
        finalBaseUrl = removeTrailingSlash(localBaseUrl) ?? baseUrl;
      }
      let querySerializer = typeof globalQuerySerializer === "function" ? globalQuerySerializer : createQuerySerializer(globalQuerySerializer);
      if (requestQuerySerializer) {
        querySerializer = typeof requestQuerySerializer === "function" ? requestQuerySerializer : createQuerySerializer({
          ...typeof globalQuerySerializer === "object" ? globalQuerySerializer : {},
          ...requestQuerySerializer
        });
      }
      const pathSerializer = requestPathSerializer || globalPathSerializer || defaultPathSerializer;
      const serializedBody = body === void 0 ? void 0 : bodySerializer(
        body,
        // Note: we declare mergeHeaders() both here and below because it’s a bit of a chicken-or-egg situation:
        // bodySerializer() needs all headers so we aren’t dropping ones set by the user, however,
        // the result of this ALSO sets the lowest-priority content-type header. So we re-merge below,
        // setting the content-type at the very beginning to be overwritten.
        // Lastly, based on the way headers work, it’s not a simple “present-or-not” check becauase null intentionally un-sets headers.
        mergeHeaders(baseHeaders, headers, params.header)
      );
      const finalHeaders = mergeHeaders(
        // with no body, we should not to set Content-Type
        serializedBody === void 0 || // if serialized body is FormData; browser will correctly set Content-Type & boundary expression
        serializedBody instanceof FormData ? {} : {
          "Content-Type": "application/json"
        },
        baseHeaders,
        headers,
        params.header
      );
      const finalMiddlewares = [...globalMiddlewares, ...requestMiddlewares];
      const requestInit = {
        redirect: "follow",
        ...baseOptions,
        ...init,
        body: serializedBody,
        headers: finalHeaders
      };
      let id;
      let options;
      let request = new Request(
        createFinalURL(schemaPath, { baseUrl: finalBaseUrl, params, querySerializer, pathSerializer }),
        requestInit
      );
      let response;
      for (const key in init) {
        if (!(key in request)) {
          request[key] = init[key];
        }
      }
      if (finalMiddlewares.length) {
        id = randomID();
        options = Object.freeze({
          baseUrl: finalBaseUrl,
          fetch: fetch2,
          parseAs,
          querySerializer,
          bodySerializer,
          pathSerializer
        });
        for (const m of finalMiddlewares) {
          if (m && typeof m === "object" && typeof m.onRequest === "function") {
            const result = await m.onRequest({
              request,
              schemaPath,
              params,
              options,
              id
            });
            if (result) {
              if (result instanceof Request) {
                request = result;
              } else if (result instanceof Response) {
                response = result;
                break;
              } else {
                throw new Error("onRequest: must return new Request() or Response() when modifying the request");
              }
            }
          }
        }
      }
      if (!response) {
        try {
          response = await fetch2(request, requestInitExt);
        } catch (error2) {
          let errorAfterMiddleware = error2;
          if (finalMiddlewares.length) {
            for (let i = finalMiddlewares.length - 1; i >= 0; i--) {
              const m = finalMiddlewares[i];
              if (m && typeof m === "object" && typeof m.onError === "function") {
                const result = await m.onError({
                  request,
                  error: errorAfterMiddleware,
                  schemaPath,
                  params,
                  options,
                  id
                });
                if (result) {
                  if (result instanceof Response) {
                    errorAfterMiddleware = void 0;
                    response = result;
                    break;
                  }
                  if (result instanceof Error) {
                    errorAfterMiddleware = result;
                    continue;
                  }
                  throw new Error("onError: must return new Response() or instance of Error");
                }
              }
            }
          }
          if (errorAfterMiddleware) {
            throw errorAfterMiddleware;
          }
        }
        if (finalMiddlewares.length) {
          for (let i = finalMiddlewares.length - 1; i >= 0; i--) {
            const m = finalMiddlewares[i];
            if (m && typeof m === "object" && typeof m.onResponse === "function") {
              const result = await m.onResponse({
                request,
                response,
                schemaPath,
                params,
                options,
                id
              });
              if (result) {
                if (!(result instanceof Response)) {
                  throw new Error("onResponse: must return new Response() when modifying the response");
                }
                response = result;
              }
            }
          }
        }
      }
      const contentLength = response.headers.get("Content-Length");
      if (response.status === 204 || request.method === "HEAD" || contentLength === "0" && !response.headers.get("Transfer-Encoding")?.includes("chunked")) {
        return response.ok ? { data: void 0, response } : { error: void 0, response };
      }
      if (response.ok) {
        const getResponseData = async () => {
          if (parseAs === "stream") {
            return response.body;
          }
          if (parseAs === "json" && !contentLength) {
            const raw = await response.text();
            return raw ? JSON.parse(raw) : void 0;
          }
          return await response[parseAs]();
        };
        return { data: await getResponseData(), response };
      }
      let error = await response.text();
      try {
        error = JSON.parse(error);
      } catch {
      }
      return { error, response };
    }
    return {
      request(method, url, init) {
        return coreFetch(url, { ...init, method: method.toUpperCase() });
      },
      /** Call a GET endpoint */
      GET(url, init) {
        return coreFetch(url, { ...init, method: "GET" });
      },
      /** Call a PUT endpoint */
      PUT(url, init) {
        return coreFetch(url, { ...init, method: "PUT" });
      },
      /** Call a POST endpoint */
      POST(url, init) {
        return coreFetch(url, { ...init, method: "POST" });
      },
      /** Call a DELETE endpoint */
      DELETE(url, init) {
        return coreFetch(url, { ...init, method: "DELETE" });
      },
      /** Call a OPTIONS endpoint */
      OPTIONS(url, init) {
        return coreFetch(url, { ...init, method: "OPTIONS" });
      },
      /** Call a HEAD endpoint */
      HEAD(url, init) {
        return coreFetch(url, { ...init, method: "HEAD" });
      },
      /** Call a PATCH endpoint */
      PATCH(url, init) {
        return coreFetch(url, { ...init, method: "PATCH" });
      },
      /** Call a TRACE endpoint */
      TRACE(url, init) {
        return coreFetch(url, { ...init, method: "TRACE" });
      },
      /** Register middleware */
      use(...middleware) {
        for (const m of middleware) {
          if (!m) {
            continue;
          }
          if (typeof m !== "object" || !("onRequest" in m || "onResponse" in m || "onError" in m)) {
            throw new Error("Middleware must be an object with one of `onRequest()`, `onResponse() or `onError()`");
          }
          globalMiddlewares.push(m);
        }
      },
      /** Unregister middleware */
      eject(...middleware) {
        for (const m of middleware) {
          const i = globalMiddlewares.indexOf(m);
          if (i !== -1) {
            globalMiddlewares.splice(i, 1);
          }
        }
      }
    };
  }
  function serializePrimitiveParam(name, value, options) {
    if (value === void 0 || value === null) {
      return "";
    }
    if (typeof value === "object") {
      throw new Error(
        "Deeply-nested arrays/objects aren\u2019t supported. Provide your own `querySerializer()` to handle these."
      );
    }
    return `${name}=${options?.allowReserved === true ? value : encodeURIComponent(value)}`;
  }
  function serializeObjectParam(name, value, options) {
    if (!value || typeof value !== "object") {
      return "";
    }
    const values = [];
    const joiner = {
      simple: ",",
      label: ".",
      matrix: ";"
    }[options.style] || "&";
    if (options.style !== "deepObject" && options.explode === false) {
      for (const k in value) {
        values.push(k, options.allowReserved === true ? value[k] : encodeURIComponent(value[k]));
      }
      const final2 = values.join(",");
      switch (options.style) {
        case "form": {
          return `${name}=${final2}`;
        }
        case "label": {
          return `.${final2}`;
        }
        case "matrix": {
          return `;${name}=${final2}`;
        }
        default: {
          return final2;
        }
      }
    }
    for (const k in value) {
      const finalName = options.style === "deepObject" ? `${name}[${k}]` : k;
      values.push(serializePrimitiveParam(finalName, value[k], options));
    }
    const final = values.join(joiner);
    return options.style === "label" || options.style === "matrix" ? `${joiner}${final}` : final;
  }
  function serializeArrayParam(name, value, options) {
    if (!Array.isArray(value)) {
      return "";
    }
    if (options.explode === false) {
      const joiner2 = { form: ",", spaceDelimited: "%20", pipeDelimited: "|" }[options.style] || ",";
      const final = (options.allowReserved === true ? value : value.map((v) => encodeURIComponent(v))).join(joiner2);
      switch (options.style) {
        case "simple": {
          return final;
        }
        case "label": {
          return `.${final}`;
        }
        case "matrix": {
          return `;${name}=${final}`;
        }
        // case "spaceDelimited":
        // case "pipeDelimited":
        default: {
          return `${name}=${final}`;
        }
      }
    }
    const joiner = { simple: ",", label: ".", matrix: ";" }[options.style] || "&";
    const values = [];
    for (const v of value) {
      if (options.style === "simple" || options.style === "label") {
        values.push(options.allowReserved === true ? v : encodeURIComponent(v));
      } else {
        values.push(serializePrimitiveParam(name, v, options));
      }
    }
    return options.style === "label" || options.style === "matrix" ? `${joiner}${values.join(joiner)}` : values.join(joiner);
  }
  function createQuerySerializer(options) {
    return function querySerializer(queryParams) {
      const search = [];
      if (queryParams && typeof queryParams === "object") {
        for (const name in queryParams) {
          const value = queryParams[name];
          if (value === void 0 || value === null) {
            continue;
          }
          if (Array.isArray(value)) {
            if (value.length === 0) {
              continue;
            }
            search.push(
              serializeArrayParam(name, value, {
                style: "form",
                explode: true,
                ...options?.array,
                allowReserved: options?.allowReserved || false
              })
            );
            continue;
          }
          if (typeof value === "object") {
            search.push(
              serializeObjectParam(name, value, {
                style: "deepObject",
                explode: true,
                ...options?.object,
                allowReserved: options?.allowReserved || false
              })
            );
            continue;
          }
          search.push(serializePrimitiveParam(name, value, options));
        }
      }
      return search.join("&");
    };
  }
  function defaultPathSerializer(pathname, pathParams) {
    let nextURL = pathname;
    for (const match of pathname.match(PATH_PARAM_RE) ?? []) {
      let name = match.substring(1, match.length - 1);
      let explode = false;
      let style = "simple";
      if (name.endsWith("*")) {
        explode = true;
        name = name.substring(0, name.length - 1);
      }
      if (name.startsWith(".")) {
        style = "label";
        name = name.substring(1);
      } else if (name.startsWith(";")) {
        style = "matrix";
        name = name.substring(1);
      }
      if (!pathParams || pathParams[name] === void 0 || pathParams[name] === null) {
        continue;
      }
      const value = pathParams[name];
      if (Array.isArray(value)) {
        nextURL = nextURL.replace(match, serializeArrayParam(name, value, { style, explode }));
        continue;
      }
      if (typeof value === "object") {
        nextURL = nextURL.replace(match, serializeObjectParam(name, value, { style, explode }));
        continue;
      }
      if (style === "matrix") {
        nextURL = nextURL.replace(match, `;${serializePrimitiveParam(name, value)}`);
        continue;
      }
      nextURL = nextURL.replace(match, style === "label" ? `.${encodeURIComponent(value)}` : encodeURIComponent(value));
    }
    return nextURL;
  }
  function defaultBodySerializer(body, headers) {
    if (body instanceof FormData) {
      return body;
    }
    if (headers) {
      const contentType = headers.get instanceof Function ? headers.get("Content-Type") ?? headers.get("content-type") : headers["Content-Type"] ?? headers["content-type"];
      if (contentType === "application/x-www-form-urlencoded") {
        return new URLSearchParams(body).toString();
      }
    }
    return JSON.stringify(body);
  }
  function createFinalURL(pathname, options) {
    let finalURL = `${options.baseUrl}${pathname}`;
    if (options.params?.path) {
      finalURL = options.pathSerializer(finalURL, options.params.path);
    }
    let search = options.querySerializer(options.params.query ?? {});
    if (search.startsWith("?")) {
      search = search.substring(1);
    }
    if (search) {
      finalURL += `?${search}`;
    }
    return finalURL;
  }
  function mergeHeaders(...allHeaders) {
    const finalHeaders = new Headers();
    for (const h of allHeaders) {
      if (!h || typeof h !== "object") {
        continue;
      }
      const iterator = h instanceof Headers ? h.entries() : Object.entries(h);
      for (const [k, v] of iterator) {
        if (v === null) {
          finalHeaders.delete(k);
        } else if (Array.isArray(v)) {
          for (const v2 of v) {
            finalHeaders.append(k, v2);
          }
        } else if (v !== void 0) {
          finalHeaders.set(k, v);
        }
      }
    }
    return finalHeaders;
  }
  function removeTrailingSlash(url) {
    if (url.endsWith("/")) {
      return url.substring(0, url.length - 1);
    }
    return url;
  }

  // frontend/api.ts
  var GatewayError = class extends Error {
    status;
    constructor(status) {
      super("Weather is temporarily unavailable. Try again.");
      this.name = "GatewayError";
      this.status = status;
    }
  };
  function createWeatherClient(baseUrl, fetcher = fetch) {
    const client = createClient({ baseUrl, fetch: fetcher });
    client.use({
      async onResponse({ response }) {
        if (response.ok) return response;
        try {
          await response.clone().json();
          return response;
        } catch {
          throw new GatewayError(response.status);
        }
      }
    });
    return client;
  }
  var LatestRequest = class {
    controller = null;
    start() {
      this.cancel();
      this.controller = new AbortController();
      return this.controller;
    }
    cancel() {
      this.controller?.abort();
      this.controller = null;
    }
    isCurrent(controller) {
      return this.controller === controller && !controller.signal.aborted;
    }
  };

  // web/weather.ts
  var api = createWeatherClient(location.origin);
  var searchRequests = new LatestRequest();
  function $(id) {
    const node = document.getElementById(id);
    if (!node) throw new Error(`Missing page element: ${id}`);
    return node;
  }
  function input(id) {
    const node = $(id);
    if (!(node instanceof HTMLInputElement)) throw new Error(`Invalid input: ${id}`);
    return node;
  }
  function select(id) {
    const node = $(id);
    if (!(node instanceof HTMLSelectElement)) throw new Error(`Invalid select: ${id}`);
    return node;
  }
  function button(id) {
    const node = $(id);
    if (!(node instanceof HTMLButtonElement)) throw new Error(`Invalid button: ${id}`);
    return node;
  }
  function link(id) {
    const node = $(id);
    if (!(node instanceof HTMLAnchorElement)) throw new Error(`Invalid link: ${id}`);
    return node;
  }
  var cityInput = input("city");
  var unitsInput = select("units");
  var go = button("go");
  var jsonLink = link("jsonLink");
  var selected = null;
  var lastQuery = { city: cityInput.value };
  var timer;
  var reportZone = "UTC";
  var text = (id, value) => {
    $(id).textContent = value;
  };
  function el(tag, className, value) {
    const node = document.createElement(tag);
    if (className) node.className = className;
    if (value !== void 0) node.textContent = value;
    return node;
  }
  function units() {
    return unitsInput.value === "metric" ? "metric" : "us";
  }
  function time(value, options = {}) {
    if (!value) return "Unavailable";
    const date = new Date(value);
    if (Number.isNaN(date.getTime())) return "Unavailable";
    return new Intl.DateTimeFormat("en-US", { timeZone: reportZone, ...options }).format(date);
  }
  function quantity(q) {
    return q && typeof q.value === "number" ? `${Math.round(q.value)} ${q.unit}` : "Unavailable";
  }
  function developer(query) {
    const params = new URLSearchParams();
    for (const [key, value] of Object.entries({ ...query, units: units() })) {
      if (value !== void 0) params.set(key, String(value));
    }
    const path = "/v1/weather?" + params;
    text("curl", `curl '${location.origin}${path}'`);
    jsonLink.href = path;
    text("mcpUrl", location.origin + "/mcp");
  }
  function choices(cities) {
    $("suggestions").replaceChildren();
    for (const city of cities) {
      const button2 = el("button", null, `${city.name}, ${city.state}`);
      button2.type = "button";
      button2.append(el("span", null, `${city.stateName} \xB7 ${city.country}`));
      button2.addEventListener("click", () => {
        selected = city;
        cityInput.value = `${city.name}, ${city.state}`;
        cancelSearch();
        closeChoices();
        void load({ cityId: city.id });
      });
      $("suggestions").append(button2);
    }
    $("suggestions").hidden = !cities.length;
    cityInput.setAttribute("aria-expanded", String(Boolean(cities.length)));
  }
  function closeChoices() {
    $("suggestions").hidden = true;
    cityInput.setAttribute("aria-expanded", "false");
  }
  function cancelSearch() {
    clearTimeout(timer);
    searchRequests.cancel();
  }
  cityInput.addEventListener("input", () => {
    selected = null;
    cancelSearch();
    timer = setTimeout(async () => {
      if (cityInput.value.trim().length < 2) {
        closeChoices();
        return;
      }
      const controller = searchRequests.start();
      try {
        const { data } = await api.GET("/v1/cities", {
          params: { query: { q: cityInput.value } },
          signal: controller.signal
        });
        if (searchRequests.isCurrent(controller)) choices(data?.data || []);
      } catch {
        if (searchRequests.isCurrent(controller)) closeChoices();
      }
    }, 200);
  });
  cityInput.addEventListener("keydown", (event) => {
    if (event.key === "Escape") closeChoices();
    if (event.key === "ArrowDown" && !$("suggestions").hidden) {
      event.preventDefault();
      $("suggestions").querySelector("button")?.focus();
    }
  });
  $("suggestions").addEventListener("keydown", (event) => {
    const buttons = [...$("suggestions").querySelectorAll("button")];
    const index = buttons.findIndex((button2) => button2 === document.activeElement);
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      buttons[(index + (event.key === "ArrowDown" ? 1 : -1) + buttons.length) % buttons.length]?.focus();
    }
    if (event.key === "Escape") {
      closeChoices();
      cityInput.focus();
    }
  });
  $("search").addEventListener("submit", (event) => {
    event.preventDefault();
    cancelSearch();
    closeChoices();
    void load(selected ? { cityId: selected.id } : { city: cityInput.value.trim() });
  });
  unitsInput.addEventListener("change", () => {
    void load(lastQuery);
  });
  var REFRESH_MS = 12e4;
  var REQUEST_TIMEOUT_MS = 55e3;
  var refreshTimer;
  var expiryTimer;
  var displayedReport = null;
  var weatherController = null;
  var freshUntil = 0;
  var reportExpired = false;
  function freshness() {
    if (!displayedReport) return;
    const expired = reportExpired || Date.now() >= freshUntil;
    text(
      "freshness",
      `Report assembled ${time(displayedReport.assembledAt, { hour: "numeric", minute: "2-digit", second: "2-digit" })} \xB7 ${expired ? "Weather needs refresh" : "Refreshes automatically while this tab is visible"}`
    );
    const badge = $("alertBadge"), d = displayedReport;
    badge.className = "tag" + (expired || d.alerts.length || d.alertsStatus !== "checked" ? " warning" : "");
    badge.textContent = expired ? d.alerts.length ? `Previous alerts \xB7 ${d.alerts.length}` : "Alerts need refresh" : d.alertsStatus !== "checked" ? "Alerts unavailable" : d.alerts.length ? `${d.alerts.length} active alert${d.alerts.length === 1 ? "" : "s"}` : "No active alerts";
    const c = d.current;
    if (c) {
      const observed = Date.parse(c.observedAt);
      const stale = c.stale || !Number.isFinite(observed) || Date.now() - observed > 72e5;
      text(
        "observed",
        `${stale ? "Older observation \xB7 " : ""}${time(c.observedAt, { weekday: "short", hour: "numeric", minute: "2-digit" })} \xB7 Station ${c.station}, ${c.stationDistanceKm} km away`
      );
    }
  }
  function scheduleRefresh(delay = REFRESH_MS) {
    clearTimeout(refreshTimer);
    clearTimeout(expiryTimer);
    if (displayedReport && !reportExpired && freshUntil > Date.now()) {
      expiryTimer = setTimeout(freshness, freshUntil - Date.now());
    }
    refreshTimer = setTimeout(() => {
      freshness();
      if (!document.hidden && !weatherController) void load(lastQuery, { refresh: true });
    }, delay);
  }
  document.addEventListener("visibilitychange", () => {
    freshness();
    if (!document.hidden && displayedReport && !weatherController && (reportExpired || Date.now() >= freshUntil))
      void load(lastQuery, { refresh: true });
  });
  async function load(query, { refresh = false } = {}) {
    clearTimeout(refreshTimer);
    weatherController?.abort();
    const controller = new AbortController();
    weatherController = controller;
    let timedOut = false;
    let failureMessage = "Weather is temporarily unavailable. Try again.";
    const requestTimer = setTimeout(() => {
      timedOut = true;
      controller.abort();
    }, REQUEST_TIMEOUT_MS);
    lastQuery = query;
    developer(query);
    go.disabled = true;
    text(
      "status",
      refresh ? "Refreshing weather and official alerts\u2026" : "Checking the forecast, nearby stations, and official alerts\u2026"
    );
    $("status").className = "status";
    if (!refresh) {
      clearTimeout(expiryTimer);
      displayedReport = null;
      $("weather").hidden = true;
    }
    $("empty").hidden = true;
    freshness();
    try {
      const {
        data,
        error,
        response: res
      } = await api.GET("/v1/weather", {
        params: { query: { ...query, units: units() } },
        signal: controller.signal
      });
      if (controller !== weatherController) return;
      if (!res.ok || !data) {
        const first = error?.errors?.[0];
        if (!refresh) choices(first?.choices || []);
        failureMessage = first?.detail || failureMessage;
        throw new Error(failureMessage);
      }
      const body = data;
      render(body.data);
      displayedReport = body.data;
      reportExpired = false;
      const cacheControl = res.headers.get("Cache-Control") || "";
      const maxAge = cacheControl.match(/(?:^|[,\s])max-age=(\d+)/);
      const age = Math.max(0, Number(res.headers.get("Age")) || 0);
      const remaining = maxAge ? Math.max(0, Number(maxAge[1]) - age) * 1e3 : REFRESH_MS;
      const assembled = Date.parse(body.data.assembledAt);
      freshUntil = Math.min(
        Date.now() + remaining,
        Number.isFinite(assembled) ? assembled + REFRESH_MS : Date.now()
      );
      freshness();
      text("status", "");
      $("weather").hidden = false;
      scheduleRefresh(Math.max(1e4, Math.min(REFRESH_MS, freshUntil - Date.now())));
    } catch (e) {
      if (controller !== weatherController) return;
      if (!(e instanceof Error) || e.name !== "AbortError" || timedOut) {
        if (refresh && displayedReport) {
          reportExpired = true;
          freshness();
          scheduleRefresh();
        }
        text(
          "status",
          `${timedOut ? "Weather refresh timed out. Please retry." : failureMessage}${refresh ? " Displayed weather may be out of date; retrying automatically." : ""}`
        );
        $("status").className = "status error";
      }
    } finally {
      clearTimeout(requestTimer);
      if (controller === weatherController) {
        weatherController = null;
        go.disabled = false;
      }
    }
  }
  function render(d) {
    reportZone = d.location.timeZone || "UTC";
    text("location", d.location.name);
    text(
      "locationNote",
      `${d.location.precision === "city-center" ? "City center" : "Selected coordinates"} \xB7 ${d.location.latitude.toFixed(3)}, ${d.location.longitude.toFixed(3)} \xB7 ${reportZone}`
    );
    const current = d.current;
    if (current) {
      $("temperature").replaceChildren(
        el(
          "span",
          null,
          current.temperature ? String(Math.round(current.temperature.value)) : "\u2014"
        ),
        el("small", null, current.temperature?.unit || "")
      );
      text("condition", current.condition || "Conditions unavailable");
      text("wind", quantity(current.windSpeed));
      text(
        "humidity",
        typeof current.humidityPercent === "number" ? `${Math.round(current.humidityPercent)}%` : "Unavailable"
      );
    } else {
      text("temperature", "\u2014");
      text("condition", "Observation unavailable");
      text("wind", "\u2014");
      text("humidity", "\u2014");
      text("observed", "See the forecast alongside.");
    }
    const firstPeriod = d.forecast[0];
    text(
      "outlookTitle",
      firstPeriod ? `${firstPeriod.name}: ${firstPeriod.condition}` : "Forecast"
    );
    text("summary", d.summary);
    $("proseUnits").hidden = d.units !== "metric";
    text(
      "issued",
      `Forecast issued ${time(d.sources.forecast.issuedAt, { month: "short", day: "numeric", hour: "numeric", minute: "2-digit" })}`
    );
    $("alerts").replaceChildren();
    for (const alert of d.alerts) {
      const details = el("details", "alert");
      details.append(
        el("summary", null, alert.headline || alert.event),
        el("pre", null, alert.description || ""),
        el("pre", null, alert.instruction || "")
      );
      $("alerts").append(details);
    }
    $("warnings").replaceChildren(...d.warnings.map((warning) => el("p", null, warning)));
    renderHours(d.hourly);
    $("days").replaceChildren();
    for (const period of d.forecast.filter((period2) => period2.isDaytime).slice(0, 4)) {
      const card = el("article", "day");
      card.append(
        el("h4", null, period.name || "Forecast"),
        el("div", "value", quantity(period.temperature)),
        el("p", null, period.condition),
        el("p", null, `Wind ${period.wind}`)
      );
      $("days").append(card);
    }
  }
  function renderHours(periods) {
    $("chart").replaceChildren();
    $("hours").replaceChildren();
    if (!periods.length) {
      text("chart", "Hourly forecast is unavailable.");
      return;
    }
    const selected2 = periods.filter((_, index) => index % 3 === 0).slice(0, 8);
    for (const period of selected2) {
      const cell = el("div", "hour", time(period.startsAt, { hour: "numeric" }));
      cell.append(
        el(
          "strong",
          null,
          period.temperature ? `${Math.round(period.temperature.value)}\xB0` : "\u2014"
        ),
        el(
          "div",
          "rain",
          period.precipitationProbabilityPercent === null ? "Rain chance unknown" : `${period.precipitationProbabilityPercent}% rain`
        )
      );
      cell.title = period.condition || "Conditions unavailable";
      $("hours").append(cell);
    }
    const values = selected2.map((period) => period.temperature?.value);
    if (!values.every((value) => typeof value === "number")) return;
    const ns = "http://www.w3.org/2000/svg";
    const svg = document.createElementNS(ns, "svg");
    svg.setAttribute("viewBox", "0 0 800 130");
    svg.setAttribute("role", "img");
    svg.setAttribute("aria-label", "Hourly temperature trend");
    const low = Math.min(...values) - 3;
    const high = Math.max(...values) + 3;
    const points = values.map((value, index) => [
      50 + index * 100,
      105 - (value - low) / (high - low) * 80
    ]);
    const area = document.createElementNS(ns, "path");
    area.setAttribute(
      "d",
      `M50 130 L${points.map((point) => point.join(" ")).join(" L")} L${50 + (values.length - 1) * 100} 130 Z`
    );
    area.setAttribute("fill", "#e5f2f7");
    svg.append(area);
    const line = document.createElementNS(ns, "polyline");
    line.setAttribute("points", points.map((point) => point.join(",")).join(" "));
    line.setAttribute("fill", "none");
    line.setAttribute("stroke", "#076a93");
    line.setAttribute("stroke-width", "2.5");
    svg.append(line);
    for (const [x, y] of points) {
      const dot = document.createElementNS(ns, "circle");
      dot.setAttribute("cx", String(x));
      dot.setAttribute("cy", String(y));
      dot.setAttribute("r", "4");
      dot.setAttribute("fill", "#076a93");
      svg.append(dot);
    }
    $("chart").append(svg);
  }
  developer(lastQuery);
})();

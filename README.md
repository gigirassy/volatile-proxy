# Volatile Proxy

Keep a pool of tor circuits and continuously rank them according to their speed and if popular sites (like duckduckgo or google) have blocked them. Then expose the best one at any given time as a SOCKS proxy from a single endpoint. Created for use with my SearXNG instance, for quick and consistent search results from a rolling ip address.

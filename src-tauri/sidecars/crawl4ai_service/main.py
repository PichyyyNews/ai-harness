"""
Crawl4AI Local Sidecar Service for AI Harness Web Grounding Engine.
Provides high-speed JS-rendered web crawling, quantitative table extraction,
and clean Markdown (.md) document generation.
"""

import asyncio
from typing import List, Optional
from fastapi import FastAPI, HTTPException
from pydantic import BaseModel

try:
    from crawl4ai import AsyncWebCrawler, CrawlerRunConfig, CacheMode
    CRAWL4AI_AVAILABLE = True
except ImportError:
    CRAWL4AI_AVAILABLE = False

app = FastAPI(title="Crawl4AI Sidecar Service", version="1.0.0")


class CrawlRequest(BaseModel):
    urls: List[str]
    word_count_threshold: Optional[int] = 10
    extract_tables: Optional[bool] = True


class CrawlResultItem(BaseModel):
    url: str
    markdown: str
    success: bool
    error_message: Optional[str] = None


class CrawlResponse(BaseModel):
    results: List[CrawlResultItem]


@app.get("/health")
def health_check():
    return {
        "status": "ok",
        "crawl4ai_installed": CRAWL4AI_AVAILABLE
    }


@app.post("/crawl", response_model=CrawlResponse)
async def crawl_urls(req: CrawlRequest):
    if not CRAWL4AI_AVAILABLE:
        raise HTTPException(
            status_code=503,
            detail="crawl4ai package is not installed in the python sidecar environment."
        )

    if not req.urls:
        return CrawlResponse(results=[])

    config = CrawlerRunConfig(
        cache_mode=CacheMode.BYPASS,
        word_count_threshold=req.word_count_threshold,
        page_timeout=10000,
    )

    items: List[CrawlResultItem] = []
    async with AsyncWebCrawler() as crawler:
        for url in req.urls[:8]:
            try:
                res = await crawler.arun(url=url, config=config)
                md_content = ""
                if res and res.markdown:
                    md_content = res.markdown.fit_markdown or res.markdown.raw_markdown or ""
                
                items.append(CrawlResultItem(
                    url=url,
                    markdown=md_content,
                    success=res.success if res else False,
                    error_message=res.error_message if res else None
                ))
            except Exception as e:
                items.append(CrawlResultItem(
                    url=url,
                    markdown="",
                    success=False,
                    error_message=str(e)
                ))

    return CrawlResponse(results=items)


if __name__ == "__main__":
    import uvicorn
    uvicorn.run(app, host="127.0.0.1", port=11235)

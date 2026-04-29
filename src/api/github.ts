import type { UseQueryOptions } from '@tanstack/react-query'

import { useQuery } from '@tanstack/react-query'
import axios from 'axios'

type GithubRepo = {
  full_name: string
  description: string
  stargazers_count: number
  forks_count: number
  open_issues_count: number
  language: string
  updated_at: string
}

const github = axios.create({
  baseURL: 'https://api.github.com',
  timeout: 10_000,
})

const githubKeys = {
  all: ['github'] as const,
  repo: (owner: string, name: string) => [...githubKeys.all, 'repo', owner, name] as const,
}

async function fetchRepo(owner: string, name: string): Promise<GithubRepo> {
  const { data } = await github.get<GithubRepo>(`/repos/${owner}/${name}`)
  return data
}

export function useGithubRepo<TData = GithubRepo>(
  owner: string,
  name: string,
  options?: Omit<UseQueryOptions<GithubRepo, Error, TData>, 'queryFn' | 'queryKey'>
) {
  return useQuery({
    queryKey: githubKeys.repo(owner, name),
    queryFn: () => fetchRepo(owner, name),
    staleTime: 5 * 60_000,
    ...options,
  })
}
